//! A buffered answer is judged with the same rules as a streamed one (spec: `response-signals`, change
//! `record-response-quality-signals`, ADR 0030): the aggregated response's content blocks and normalized stop
//! reason, against a fake upstream that answers with a whole body.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::time::Duration;

use axum::Router;
use axum::routing::post;
use common::{serve, usage};
use humpyard::config::Config;
use humpyard::ledger::Ledger;
use humpyard::server;
use humpyard::signals::Signals;
use serde_json::{Value, json};

/// A chat-completion body whose one choice is `message` and ends with `finish`.
fn answer(message: &Value, finish: &str) -> Value {
    json!({"id": "c", "object": "chat.completion", "created": 1, "model": "m",
           "choices": [{"index": 0, "message": message, "finish_reason": finish}],
           "usage": usage(10, 5)})
}

fn text(content: &str) -> Value {
    json!({"role": "assistant", "content": content})
}

fn tool(name: &str, arguments: &str) -> Value {
    json!({"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function",
           "function": {"name": name, "arguments": arguments}}]})
}

/// The signals the ledger holds for the answer to one request served with `body`.
async fn signals_of(body: Value) -> Option<Signals> {
    let app = Router::new().route(
        "/chat/completions",
        post(move || {
            let body = body.clone();
            async move { axum::Json(body) }
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
    let status = reqwest::Client::new()
        .post(format!("{url}/v1/chat/completions"))
        .json(&json!({"model": "r", "messages": [{"role": "user", "content": "hi"}]}))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, 200);
    let ledger = Ledger::open(&dir.path().join("ledger.db")).await.unwrap();
    for _ in 0..200 {
        if let Some(entry) = ledger.entries_since(0).await.unwrap().into_iter().next() {
            return entry.signals;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    panic!("no usage row was written");
}

fn only(f: impl FnOnce(&mut Signals)) -> Signals {
    let mut s = Signals::default();
    f(&mut s);
    s
}

#[tokio::test]
async fn a_good_answer_and_a_good_tool_call_carry_no_flag() {
    assert_eq!(
        signals_of(answer(&text("hello"), "stop")).await,
        Some(Signals::default())
    );
    assert_eq!(
        signals_of(answer(
            &tool("get_weather", r#"{"city":"Paris"}"#),
            "tool_calls"
        ))
        .await,
        Some(Signals::default())
    );
}

#[tokio::test]
async fn a_tool_call_with_blank_arguments_is_not_bad() {
    assert_eq!(
        signals_of(answer(&tool("get_weather", ""), "tool_calls")).await,
        Some(Signals::default())
    );
}

#[tokio::test]
async fn an_empty_answer_is_empty() {
    assert_eq!(
        signals_of(answer(&text(""), "stop")).await,
        Some(only(|s| s.empty_output = true))
    );
}

#[tokio::test]
async fn a_length_stop_is_truncated() {
    assert_eq!(
        signals_of(answer(&text("partial"), "length")).await,
        Some(only(|s| s.truncated_output = true))
    );
}

#[tokio::test]
async fn a_cut_off_tool_call_is_bad() {
    assert_eq!(
        signals_of(answer(
            &tool("get_weather", r#"{"city":"Par"#),
            "tool_calls"
        ))
        .await,
        Some(only(|s| s.bad_tool_call = true))
    );
}

#[tokio::test]
async fn a_buffered_answer_is_never_a_cut_stream() {
    let s = signals_of(answer(&text("hello"), "stop")).await.unwrap();
    assert!(!s.cut_stream);
}
