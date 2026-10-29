//! The stream tap observes an answer as it passes and records the four facts (spec: `response-signals`, change
//! `record-response-quality-signals`, ADR 0030), against a fake streaming upstream: a whole answer, one that stops
//! early, one that errors and a client that disconnects. The client receives the provider's events untouched.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::time::Duration;

use axum::Router;
use axum::response::sse::{Event, Sse};
use axum::routing::post;
use common::{chunk, serve};
use futures::StreamExt;
use humpyard::config::Config;
use humpyard::ledger::{Entry, Ledger};
use humpyard::server;
use humpyard::signals::Signals;
use serde_json::{Value, json};

/// What the fake upstream does.
#[derive(Clone, Copy)]
enum Script {
    /// Text, a normal finish, usage and `[DONE]`.
    Whole,
    /// Text, then the body just ends: no finish chunk, no `[DONE]`.
    StopsEarly,
    /// Text, then the connection breaks.
    Errors,
    /// Text, then silence for a long time (the client leaves meanwhile).
    Slow,
    /// A tool call whose arguments are `GOOD` or `CUT`, a `tool_calls` finish and `[DONE]`.
    ToolCall(&'static str),
    /// Text, then a `length` finish.
    Length,
    /// A finish with neither text nor a tool call.
    Empty,
}

const GOOD: &str = r#"{\"city\":\"Paris\"}"#;
const CUT: &str = r#"{\"city\":\"Par"#;

fn tool_chunk(arguments: &str) -> Value {
    chunk(
        json!({"tool_calls": [{"index": 0, "id": "call_1", "type": "function",
               "function": {"name": "get_weather", "arguments": arguments}}]}),
        None,
    )
}

/// A gateway with one passthrough route `r` over a streaming upstream playing `script`, and the ledger's path.
async fn gateway(script: Script) -> (String, tempfile::TempDir) {
    let app = Router::new().route(
        "/chat/completions",
        post(move || async move {
            let events = async_stream::stream! {
                let event = |v: Value| Event::default().data(v.to_string());
                match script {
                    Script::ToolCall(arguments) => {
                        let raw = tool_chunk("ARGS").to_string().replace("ARGS", arguments);
                        yield Ok::<_, std::io::Error>(Event::default().data(raw));
                        yield Ok(event(chunk(json!({}), Some("tool_calls"))));
                    }
                    Script::Empty => {
                        yield Ok(event(chunk(json!({"role": "assistant"}), Some("stop"))));
                    }
                    _ => {
                        yield Ok(event(chunk(json!({"role": "assistant", "content": "Hel"}), None)));
                    }
                }
                match script {
                    Script::Whole => {
                        yield Ok(event(chunk(json!({"content": "lo"}), Some("stop"))));
                    }
                    Script::Length => {
                        yield Ok(event(chunk(json!({"content": "lo"}), Some("length"))));
                    }
                    Script::Errors => {
                        // Let the first event reach the client before the connection breaks.
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        yield Err(std::io::Error::other("connection reset"));
                    }
                    Script::Slow => tokio::time::sleep(Duration::from_secs(30)).await,
                    _ => {}
                }
                if matches!(
                    script,
                    Script::Whole | Script::Length | Script::ToolCall(_) | Script::Empty
                ) {
                    yield Ok(Event::default().data("[DONE]"));
                }
            };
            Sse::new(events)
        }),
    );
    let upstream = format!("http://{}", serve(app).await);
    let dir = tempfile::tempdir().unwrap();
    let toml = format!(
        "listen = \"127.0.0.1:0\"\n[ledger]\npath = \"{}\"\n\
         [providers.p]\nbase_url = \"{upstream}\"\napi_key_env = \"K\"\nmax_retries = 0\ntimeout_secs = 5\n\
         [targets.t]\nendpoints = [{{ provider = \"p\", model = \"m\" }}]\n\
         [routes.r]\ntype = \"passthrough\"\ntargets = [\"t\"]\n",
        dir.path().join("ledger.db").display()
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    (url, dir)
}

fn request() -> Value {
    json!({"model": "r", "stream": true, "messages": [{"role": "user", "content": "hi"}]})
}

/// The whole streamed body the client receives (it may be cut short by an upstream that breaks).
async fn body(url: &str) -> String {
    let response = reqwest::Client::new()
        .post(format!("{url}/v1/chat/completions"))
        .json(&request())
        .send()
        .await
        .unwrap();
    let mut stream = response.bytes_stream();
    let mut out = Vec::new();
    while let Some(Ok(bytes)) = stream.next().await {
        out.extend_from_slice(&bytes);
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The answer entry once the writer has stored it.
async fn answer(dir: &tempfile::TempDir) -> Entry {
    let ledger = Ledger::open(&dir.path().join("ledger.db")).await.unwrap();
    for _ in 0..200 {
        if let Some(entry) = ledger.entries_since(0).await.unwrap().into_iter().next() {
            return entry;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    panic!("no usage row was written");
}

async fn signals_of(script: Script) -> Option<Signals> {
    let (url, dir) = gateway(script).await;
    body(&url).await;
    answer(&dir).await.signals
}

#[tokio::test]
async fn a_whole_answer_is_observed_and_clean() {
    assert_eq!(signals_of(Script::Whole).await, Some(Signals::default()));
}

#[tokio::test]
async fn a_stream_that_stops_early_is_a_cut_stream() {
    let s = signals_of(Script::StopsEarly).await.unwrap();
    assert_eq!(
        s,
        Signals {
            cut_stream: true,
            ..Signals::default()
        }
    );
}

#[tokio::test]
async fn a_stream_that_errors_is_a_cut_stream() {
    let s = signals_of(Script::Errors).await.unwrap();
    assert!(s.cut_stream);
    assert!(!s.empty_output, "it held text before it broke");
}

#[tokio::test]
async fn a_length_finish_is_truncated() {
    let s = signals_of(Script::Length).await.unwrap();
    assert_eq!(
        s,
        Signals {
            truncated_output: true,
            ..Signals::default()
        }
    );
}

#[tokio::test]
async fn an_answer_with_nothing_in_it_is_empty() {
    let s = signals_of(Script::Empty).await.unwrap();
    assert_eq!(
        s,
        Signals {
            empty_output: true,
            ..Signals::default()
        }
    );
}

#[tokio::test]
async fn a_cut_off_tool_call_is_flagged_and_a_good_one_is_not() {
    assert_eq!(
        signals_of(Script::ToolCall(GOOD)).await,
        Some(Signals::default())
    );
    let s = signals_of(Script::ToolCall(CUT)).await.unwrap();
    assert_eq!(
        s,
        Signals {
            bad_tool_call: true,
            ..Signals::default()
        }
    );
}

#[tokio::test]
async fn a_client_that_disconnects_is_not_held_against_the_model() {
    let (url, dir) = gateway(Script::Slow).await;
    let response = reqwest::Client::new()
        .post(format!("{url}/v1/chat/completions"))
        .json(&request())
        .send()
        .await
        .unwrap();
    let mut stream = response.bytes_stream();
    // Take the first bytes, then leave while the provider is still streaming.
    assert!(stream.next().await.unwrap().is_ok());
    drop(stream);
    let entry = answer(&dir).await;
    assert_eq!(entry.outcome, "cancelled");
    assert_eq!(entry.signals, None, "not observed whole, so unknown");
}

#[tokio::test]
async fn the_client_receives_the_same_events_with_and_without_a_malformed_tool_call() {
    let (good_url, _a) = gateway(Script::ToolCall(GOOD)).await;
    let (cut_url, _b) = gateway(Script::ToolCall(CUT)).await;
    let good = body(&good_url).await;
    let cut = body(&cut_url).await;
    assert!(
        cut.contains(r#"{\"city\":\"Par"#),
        "the cut-off arguments reach the client as the provider sent them: {cut}"
    );
    // Same events in the same order; only the argument text differs.
    assert_eq!(good.replace(GOOD, CUT), cut);
    assert!(cut.trim_end().ends_with("[DONE]"));
}
