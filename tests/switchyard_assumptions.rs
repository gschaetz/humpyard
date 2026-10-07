//! Pins the Switchyard 0.3 behaviors our routing design relies on (see
//! openspec/changes/add-switchyard-routing/design.md, "Spike findings"). A failure after a
//! Switchyard upgrade means the design assumption changed, not necessarily that we have a bug.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use parking_lot::Mutex;
use serde_json::json;
use switchyard_libsy::{
    Algorithm, Passthrough, PickerMode, RuntimeModels, StageRouter, StageRouterConfig,
};
use switchyard_llm_client::{ClientRouter, run};
use switchyard_protocol::{
    Category, ContentBlock, LlmClientError, LlmRequest, LlmResponse, LlmResponseChunk,
    LlmResponseStreamEvent, Message, Metadata, ModelId, Request, Response, Role, RoutedLlmClient,
    ToolCall, ToolResult, WireFormat, completion_text, text_response,
};

/// Answers with its own model id and records which targets were called.
struct Echo {
    id: String,
    log: Arc<Mutex<Vec<String>>>,
    stream_then_hang: bool,
}

#[async_trait]
impl RoutedLlmClient for Echo {
    async fn call(&self, _request: Request) -> Result<Response, LlmClientError> {
        self.log.lock().push(self.id.clone());
        let llm_response = if self.stream_then_hang {
            let first = futures::stream::iter([Ok(LlmResponseStreamEvent::from(
                LlmResponseChunk::TextDelta {
                    index: 0,
                    text: "first".into(),
                },
            ))]);
            // Never completes: proves `run` returns before the answer finishes.
            LlmResponse::Stream(first.chain(futures::stream::pending()).boxed())
        } else {
            LlmResponse::Agg(text_response(None, self.id.clone()))
        };
        Ok(Response {
            llm_response,
            metadata: None,
            upstream_headers: http::HeaderMap::new(),
        })
    }
}

fn clients(ids: &[&str], log: &Arc<Mutex<Vec<String>>>, hang: bool) -> ClientRouter {
    let map: HashMap<ModelId, Arc<dyn RoutedLlmClient>> = ids
        .iter()
        .map(|id| {
            (
                ModelId::from(*id),
                Arc::new(Echo {
                    id: id.to_string(),
                    log: log.clone(),
                    stream_then_hang: hang,
                }) as Arc<dyn RoutedLlmClient>,
            )
        })
        .collect();
    ClientRouter::new(map)
}

/// Finding: `Category::Any` must list every target (algorithms validate picks against it), so
/// the helper derives it as the union of the other categories.
fn models(pairs: &[(Category, &[&str])]) -> Arc<RuntimeModels> {
    let mut map: HashMap<Category, Vec<ModelId>> = pairs
        .iter()
        .map(|(c, ids)| (c.clone(), ids.iter().map(|i| ModelId::from(*i)).collect()))
        .collect();
    if !map.contains_key(&Category::Any) {
        let mut all: Vec<ModelId> = vec![];
        for ids in map.values() {
            for id in ids {
                if !all.contains(id) {
                    all.push(id.clone());
                }
            }
        }
        map.insert(Category::Any, all);
    }
    Arc::new(RuntimeModels::new(map))
}

fn turn(failed: bool, session: &str) -> Request {
    let content = if failed {
        "fatal runtime error: out of memory"
    } else {
        "ok"
    };
    Request {
        llm_request: LlmRequest {
            model: Some("auto".into()),
            messages: vec![
                Message::text(Role::User, "fix the build"),
                Message {
                    role: Role::Assistant,
                    content: vec![ContentBlock::ToolCall(ToolCall {
                        id: "call_1".into(),
                        name: "Bash".into(),
                        arguments: json!({"command": "cargo test"}),
                    })],
                },
                Message {
                    role: Role::Tool,
                    content: vec![ContentBlock::ToolResult(ToolResult {
                        tool_call_id: "call_1".into(),
                        content: vec![ContentBlock::Text {
                            text: content.into(),
                        }],
                        is_error: Some(failed),
                    })],
                },
            ],
            ..LlmRequest::default()
        },
        raw_request: None,
        metadata: Some(Metadata {
            wire_format: Some(WireFormat::OpenAiChat),
            session_id: Some(session.into()),
            ..Default::default()
        }),
    }
}

async fn answer(
    alg: Arc<dyn Algorithm>,
    clients: ClientRouter,
    request: Request,
    models: Arc<RuntimeModels>,
) -> Result<String, String> {
    let (selected, response) = run(alg, clients, request, models, None)
        .await
        .map_err(|e| e.to_string())?;
    match response.llm_response {
        LlmResponse::Agg(agg) => Ok(format!("{selected}:{}", completion_text(&agg))),
        LlmResponse::Stream(_) => Ok(format!("{selected}:<stream>")),
    }
}

#[tokio::test]
async fn filtered_models_hide_targets() {
    let log = Arc::new(Mutex::new(vec![]));
    let c = || clients(&["a", "b", "cap", "eff"], &log, false);
    let alg: Arc<dyn Algorithm> = Arc::new(Passthrough);

    let full = answer(
        alg.clone(),
        c(),
        turn(false, "s"),
        models(&[(Category::Any, &["a", "b"])]),
    )
    .await;
    let filtered = answer(
        alg.clone(),
        c(),
        turn(false, "s"),
        models(&[(Category::Any, &["b"])]),
    )
    .await;
    let empty = answer(alg, c(), turn(false, "s"), models(&[(Category::Any, &[])])).await;
    println!("passthrough full={full:?} filtered={filtered:?} empty={empty:?}");
    assert_eq!(full.unwrap(), "a:a");
    assert_eq!(filtered.unwrap(), "b:b");
    assert!(
        empty.is_err(),
        "empty target list must be an error, not a silent pick"
    );

    // Stage router: tiers are categories, so removing `capable` is expressible per request.
    let sr: Arc<dyn Algorithm> = Arc::new(
        StageRouter::new(StageRouterConfig::new(PickerMode::EfficientFirst, 0.5)).unwrap(),
    );
    let both = models(&[
        (Category::Capable, &["cap"]),
        (Category::Efficient, &["eff"]),
    ]);
    let no_cap = models(&[(Category::Efficient, &["eff"])]);
    let failing_full = answer(sr.clone(), c(), turn(true, "fresh-1"), both).await;
    let failing_no_cap = answer(sr, c(), turn(true, "fresh-2"), no_cap).await;
    println!("stage failing: both={failing_full:?} no_capable={failing_no_cap:?}");
    assert_eq!(failing_full.unwrap(), "cap:cap");
    // Recorded in design.md: what happens when the preferred tier is filtered out.
    println!("RESULT no_capable => {failing_no_cap:?}");
}

#[tokio::test]
async fn run_returns_stream_before_completion() {
    let log = Arc::new(Mutex::new(vec![]));
    let alg: Arc<dyn Algorithm> = Arc::new(Passthrough);
    let fut = run(
        alg,
        clients(&["a"], &log, true),
        turn(false, "s"),
        models(&[(Category::Any, &["a"])]),
        None,
    );
    let (selected, response) = tokio::time::timeout(Duration::from_secs(2), fut)
        .await
        .expect("run must not wait for the stream to finish")
        .unwrap();
    assert_eq!(selected.as_str(), "a");
    let LlmResponse::Stream(mut stream) = response.llm_response else {
        panic!("expected a live stream")
    };
    let first = tokio::time::timeout(Duration::from_secs(1), stream.next())
        .await
        .unwrap();
    assert!(first.is_some());
}

#[tokio::test]
async fn session_state_persists_across_runs() {
    let log = Arc::new(Mutex::new(vec![]));
    let c = || clients(&["cap", "eff"], &log, false);
    let both = || {
        models(&[
            (Category::Capable, &["cap"]),
            (Category::Efficient, &["eff"]),
        ])
    };
    let sr: Arc<dyn Algorithm> = Arc::new(
        StageRouter::new(StageRouterConfig::new(PickerMode::EfficientFirst, 0.5)).unwrap(),
    );

    let first = answer(sr.clone(), c(), turn(true, "s1"), both())
        .await
        .unwrap();
    let held = answer(sr.clone(), c(), turn(false, "s1"), both())
        .await
        .unwrap();
    let other_session = answer(sr.clone(), c(), turn(false, "s2"), both())
        .await
        .unwrap();
    println!("first={first} same-session-clean={held} other-session-clean={other_session}");
    assert_eq!(first, "cap:cap");
    assert_eq!(
        held, "cap:cap",
        "capable hold should persist within the session"
    );
    assert_eq!(
        other_session, "eff:eff",
        "other sessions must be unaffected"
    );
}
