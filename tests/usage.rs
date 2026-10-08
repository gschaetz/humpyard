//! Usage metering end to end: every upstream call lands in the ledger, attributed and priced.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::format_push_string,
    clippy::needless_pass_by_value
)] // test scaffolding: fail loudly, favor readability

mod common;

use common::{chat_request as chat, serve, usage};
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use humpyard::auth::generate_key;
use humpyard::config::Config;
use humpyard::ledger::{Entry, Kind, Ledger};
use humpyard::server;
use serde_json::{Value, json};

/// A streamed chunk as an SSE data payload.
fn chunk(delta: Value, finish: Option<&str>) -> String {
    common::chunk(delta, finish).to_string()
}

/// Counts calls to the judge model: it answers once, then fails. Switchyard falls back to every
/// configured target (the judge included), so for a whole request to fail after the judge has
/// spent tokens, the judge itself must stop working after its first call.
async fn completions(
    State(judge_calls): State<Arc<AtomicUsize>>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let model = body["model"].as_str().unwrap().to_string();
    let repeat_judge = model == "judge-m" && judge_calls.fetch_add(1, Ordering::SeqCst) > 0;
    if model.starts_with("broken") || model == "p1-m" || repeat_judge {
        return axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    if body["stream"] == true {
        let with_usage = model == "stream-m";
        let slow = model == "slow-m";
        let events = async_stream::stream! {
            yield Ok::<_, Infallible>(Event::default().data(chunk(json!({"role": "assistant", "content": "hi"}), None)));
            if slow {
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
            yield Ok(Event::default().data(chunk(json!({}), Some("stop"))));
            if with_usage {
                yield Ok(Event::default().data(
                    json!({"id": "c", "object": "chat.completion.chunk", "created": 1, "model": "m",
                           "choices": [], "usage": usage(12, 6)}).to_string()));
            }
            yield Ok(Event::default().data("[DONE]"));
        };
        return Sse::new(events).into_response();
    }
    if model == "delay-m" {
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
    let (content, tokens) = match model.as_str() {
        "judge-m" => (
            json!({"crux": "x", "primary_rule": "SUP-1", "capability_boundary": "supported", "p_solve": 0.9}).to_string(),
            usage(100, 20),
        ),
        "fast-m" => ("fast".to_string(), usage(10, 5)),
        "p2-m" => ("p2".to_string(), usage(7, 3)),
        _ => (model.clone(), usage(1, 1)),
    };
    axum::Json(json!({"id": "c", "object": "chat.completion", "created": 1, "model": model,
        "choices": [{"index": 0, "message": {"role": "assistant", "content": content}, "finish_reason": "stop"}],
        "usage": tokens}))
    .into_response()
}

struct Harness {
    url: String,
    key: String,
    db: PathBuf,
    _dir: tempfile::TempDir,
}

const ROUTES: &str = r#"
[routes.judged]
type = "llm_classifier"
mode = "capability"
efficient = ["fast"]
capable = ["fast"]
judge = ["judge"]
[routes.slowjudged]
type = "llm_classifier"
mode = "capability"
efficient = ["delayed"]
capable = ["delayed"]
judge = ["judge"]
[routes.doomed]
type = "llm_classifier"
mode = "capability"
efficient = ["broken"]
capable = ["broken2"]
judge = ["judge"]
"#;

async fn harness() -> Harness {
    let upstream = serve(
        Router::new()
            .route("/chat/completions", post(completions))
            .with_state(Arc::new(AtomicUsize::new(0))),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ledger.db");
    let (key, hash) = generate_key().unwrap();
    let mut toml = format!(
        "listen = \"127.0.0.1:0\"\n[ledger]\npath = \"{}\"\n[keys.alice]\nsha256 = \"{hash}\"\n",
        db.display()
    );
    for p in ["p1", "p2", "mock"] {
        toml += &format!(
            "[providers.{p}]\nbase_url = \"http://{upstream}\"\napi_key_env = \"K\"\nmax_retries = 0\n"
        );
    }
    let endpoint = |provider: &str, model: &str, input: f64, output: f64| {
        format!(
            "{{ provider = \"{provider}\", model = \"{model}\", price = {{ input = {input}, output = {output} }} }}"
        )
    };
    let targets = [
        (
            "t",
            format!(
                "[{}, {}]",
                endpoint("p1", "p1-m", 1.0, 2.0),
                endpoint("p2", "p2-m", 3.0, 9.0)
            ),
        ),
        (
            "fast",
            format!("[{}]", endpoint("mock", "fast-m", 1.0, 2.0)),
        ),
        (
            "judge",
            format!("[{}]", endpoint("mock", "judge-m", 2.0, 6.0)),
        ),
        (
            "streamer",
            format!("[{}]", endpoint("mock", "stream-m", 1.0, 2.0)),
        ),
        (
            "nousage",
            format!("[{}]", endpoint("mock", "nousage-m", 1.0, 2.0)),
        ),
        (
            "slow",
            format!("[{}]", endpoint("mock", "slow-m", 1.0, 2.0)),
        ),
        (
            "delayed",
            format!("[{}]", endpoint("mock", "delay-m", 1.0, 2.0)),
        ),
        (
            "broken",
            format!("[{}]", endpoint("mock", "broken-m", 1.0, 2.0)),
        ),
        (
            "broken2",
            format!("[{}]", endpoint("mock", "broken2-m", 1.0, 2.0)),
        ),
    ];
    for (name, endpoints) in targets {
        toml += &format!("[targets.{name}]\nendpoints = {endpoints}\n");
    }
    toml += ROUTES;
    let config = Config::from_toml(&toml, |_| Some("key".into())).unwrap();
    let gateway = serve(server::router(config).await.unwrap()).await;
    Harness {
        url: format!("http://{gateway}"),
        key,
        db,
        _dir: dir,
    }
}

impl Harness {
    async fn post(&self, path: &str, body: Value) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("{}{path}", self.url))
            .bearer_auth(&self.key)
            .json(&body)
            .send()
            .await
            .unwrap()
    }

    /// Waits for the background writer, then returns every ledger entry.
    async fn entries(&self, expected: usize) -> Vec<Entry> {
        let reader = Ledger::open(&self.db).await.unwrap();
        for _ in 0..100 {
            let entries = reader.entries_since(0).await.unwrap();
            if entries.len() >= expected {
                return entries;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        reader.entries_since(0).await.unwrap()
    }
}

#[tokio::test]
async fn failover_attributes_and_prices_the_endpoint_that_served() {
    let h = harness().await;
    assert_eq!(
        h.post("/v1/chat/completions", chat("t", false))
            .await
            .status(),
        200
    );
    let entries = h.entries(1).await;
    assert_eq!(entries.len(), 1, "{entries:?}");
    let e = &entries[0];
    assert_eq!((e.provider.as_str(), e.model.as_str()), ("p2", "p2-m"));
    assert_eq!((e.input_tokens, e.output_tokens), (7, 3));
    assert_eq!(e.cost_micro_usd, 7 * 3 + 3 * 9);
    assert_eq!(e.key_id.as_deref(), Some("alice"));
    assert_eq!(
        (e.route.as_str(), e.target.as_str(), e.kind),
        ("t", "t", Kind::Answer)
    );
    assert_eq!(e.outcome, "ok");
    assert!(!e.usage_missing);
}

#[tokio::test]
async fn judge_calls_are_separate_entries_charged_to_the_same_key() {
    let h = harness().await;
    assert_eq!(
        h.post("/v1/chat/completions", chat("judged", false))
            .await
            .status(),
        200
    );
    let entries = h.entries(2).await;
    assert_eq!(entries.len(), 2, "{entries:?}");
    let judge = entries
        .iter()
        .find(|e| e.kind == Kind::Judge)
        .expect("judge entry");
    let answer = entries
        .iter()
        .find(|e| e.kind == Kind::Answer)
        .expect("answer entry");
    assert_eq!(
        (judge.model.as_str(), judge.cost_micro_usd),
        ("judge-m", 100 * 2 + 20 * 6)
    );
    assert_eq!(
        (answer.model.as_str(), answer.cost_micro_usd),
        ("fast-m", 10 + 5 * 2)
    );
    assert!(
        entries
            .iter()
            .all(|e| e.key_id.as_deref() == Some("alice") && e.route == "judged")
    );
}

async fn drain(resp: reqwest::Response) {
    let _ = resp.bytes().await;
}

#[tokio::test]
async fn streamed_answers_are_recorded_at_stream_end_in_every_protocol() {
    let h = harness().await;
    drain(h.post("/v1/chat/completions", chat("streamer", true)).await).await;
    drain(h.post("/v1/messages", json!({"model": "streamer", "stream": true, "max_tokens": 50, "messages": [{"role": "user", "content": "hi"}]})).await).await;
    drain(
        h.post(
            "/v1/responses",
            json!({"model": "streamer", "stream": true, "input": "hi"}),
        )
        .await,
    )
    .await;
    let entries = h.entries(3).await;
    assert_eq!(entries.len(), 3, "{entries:?}");
    for e in &entries {
        assert_eq!((e.input_tokens, e.output_tokens), (12, 6), "{e:?}");
        assert_eq!(e.cost_micro_usd, 12 + 6 * 2);
        assert_eq!(
            (e.kind, e.outcome.as_str(), e.usage_missing),
            (Kind::Answer, "ok", false)
        );
    }
}

#[tokio::test]
async fn a_stream_without_usage_is_recorded_with_the_missing_marker() {
    let h = harness().await;
    drain(h.post("/v1/chat/completions", chat("nousage", true)).await).await;
    let entries = h.entries(1).await;
    assert_eq!(entries.len(), 1, "{entries:?}");
    let e = &entries[0];
    assert!(e.usage_missing);
    assert_eq!(
        (e.input_tokens, e.output_tokens, e.cost_micro_usd),
        (0, 0, 0)
    );
}

#[tokio::test]
async fn a_client_disconnect_records_what_was_seen_as_cancelled() {
    let h = harness().await;
    let mut resp = h.post("/v1/chat/completions", chat("slow", true)).await;
    resp.chunk().await.unwrap().unwrap();
    drop(resp);
    let entries = h.entries(1).await;
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].outcome, "cancelled");
    assert!(entries[0].usage_missing);
}

#[tokio::test]
async fn a_failed_run_still_records_the_judge_spend() {
    let h = harness().await;
    let resp = h.post("/v1/chat/completions", chat("doomed", false)).await;
    assert!(resp.status().is_server_error(), "{}", resp.status());
    let entries = h.entries(1).await;
    assert_eq!(
        entries.len(),
        1,
        "only the judge call returned usage: {entries:?}"
    );
    assert_eq!(entries[0].kind, Kind::Judge);
    assert_eq!(entries[0].cost_micro_usd, 100 * 2 + 20 * 6);
}

#[tokio::test]
async fn a_request_cancelled_mid_flight_still_records_the_judge_call_it_paid_for() {
    let h = harness().await;
    // The judge answers at once, then the answer call sleeps; the client gives up first.
    let impatient = reqwest::Client::builder()
        .timeout(Duration::from_millis(700))
        .build()
        .unwrap();
    let result = impatient
        .post(format!("{}/v1/chat/completions", h.url))
        .bearer_auth(&h.key)
        .json(&chat("slowjudged", false))
        .send()
        .await;
    assert!(result.is_err(), "the client should have timed out");
    let entries = h.entries(1).await;
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].kind, Kind::Judge);
    assert_eq!(entries[0].cost_micro_usd, 100 * 2 + 20 * 6);
    assert_eq!(entries[0].key_id.as_deref(), Some("alice"));
}
