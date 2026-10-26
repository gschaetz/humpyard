//! A cut-off tool call in a request's history no longer makes a strict provider refuse the request (spec:
//! `upstream-proxy`, change `repair-tool-arguments`, ADR 0029). The fake upstream behaves like NVIDIA: it answers
//! 400 for any assistant tool call whose arguments are not a JSON object, and records what it was sent.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use common::{chat_completion, serve, usage};
use humpyard::config::Config;
use humpyard::server;
use serde_json::{Value, json};

const CUT: &str = r#"{"cmd":["bash","-lc","ls -la"],"workdir":"/tm"#;

/// What the upstream was sent, in order.
type Seen = Arc<Mutex<Vec<Value>>>;

/// Every assistant tool call's `arguments` string in a chat body.
fn arguments(body: &Value) -> Vec<String> {
    body["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| m["tool_calls"].as_array())
        .flatten()
        .map(|c| {
            c["function"]["arguments"]
                .as_str()
                .unwrap_or("<missing>")
                .to_string()
        })
        .collect()
}

fn is_object(text: &str) -> bool {
    serde_json::from_str::<Value>(text).is_ok_and(|v| v.is_object())
}

async fn strict_upstream(
    State(seen): State<Seen>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let args = arguments(&body);
    seen.lock().unwrap().push(body);
    if let Some(bad) = args.iter().find(|a| !is_object(a)) {
        let message = format!(
            "Validation: `messages[1].tool_calls[0].function.arguments` must be a valid JSON object string: {bad}"
        );
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(json!({"error": {"message": message}})),
        )
            .into_response();
    }
    axum::Json(chat_completion("m", "ok", Some(usage(10, 5)))).into_response()
}

/// A gateway with one passthrough route `r` over one strict upstream. Returns its URL and what the upstream saw.
async fn gateway() -> (String, Seen, tempfile::TempDir) {
    let seen: Seen = Arc::default();
    let app = Router::new()
        .route("/chat/completions", post(strict_upstream))
        .with_state(seen.clone());
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
    (url, seen, dir)
}

async fn post_json(url: &str, path: &str, body: &Value) -> u16 {
    reqwest::Client::new()
        .post(format!("{url}{path}"))
        .json(body)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

fn chat_history(call_arguments: &str) -> Value {
    json!({
        "model": "r",
        "messages": [
            {"role": "user", "content": "list files"},
            {"role": "assistant", "content": null, "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "shell", "arguments": call_arguments}}
            ]},
            {"role": "tool", "tool_call_id": "c1", "content": "ok"},
            {"role": "user", "content": "go on"}
        ]
    })
}

fn responses_history(call_arguments: &str) -> Value {
    json!({
        "model": "r",
        "stream": false,
        "input": [
            {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "list files"}]},
            {"type": "function_call", "call_id": "c1", "name": "shell", "arguments": call_arguments},
            {"type": "function_call_output", "call_id": "c1", "output": "ok"},
            {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "go on"}]}
        ]
    })
}

#[tokio::test]
async fn a_cut_off_call_in_a_chat_history_is_completed_before_the_strict_provider_sees_it() {
    let (url, seen, _dir) = gateway().await;
    assert_eq!(
        post_json(&url, "/v1/chat/completions", &chat_history(CUT)).await,
        200
    );

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1, "one upstream call, no refusal and no retry");
    let sent = arguments(&seen[0]);
    assert_eq!(sent.len(), 1);
    let object: Value = serde_json::from_str(&sent[0]).unwrap();
    assert_eq!(object["workdir"], "/tm");
    assert_eq!(object["cmd"], json!(["bash", "-lc", "ls -la"]));
}

#[tokio::test]
async fn a_cut_off_call_in_a_responses_history_is_completed_too() {
    let (url, seen, _dir) = gateway().await;
    assert_eq!(
        post_json(&url, "/v1/responses", &responses_history(CUT)).await,
        200
    );

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let sent = arguments(&seen[0]);
    assert_eq!(sent.len(), 1);
    assert_eq!(
        serde_json::from_str::<Value>(&sent[0]).unwrap()["workdir"],
        "/tm"
    );
}

#[tokio::test]
async fn arguments_that_cannot_be_completed_are_sent_as_an_empty_object() {
    let (url, seen, _dir) = gateway().await;
    assert_eq!(
        post_json(&url, "/v1/chat/completions", &chat_history("not json")).await,
        200
    );
    assert_eq!(
        post_json(&url, "/v1/responses", &responses_history("")).await,
        200
    );

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(arguments(&seen[0]), vec!["{}".to_string()]);
    assert_eq!(arguments(&seen[1]), vec!["{}".to_string()]);
}

#[tokio::test]
async fn valid_arguments_reach_the_provider_exactly_as_the_client_wrote_them() {
    let (url, seen, _dir) = gateway().await;
    let spaced = "{ \"cmd\" : [ \"ls\" ] ,\n \"workdir\":\"/tmp\" }";
    assert_eq!(
        post_json(&url, "/v1/chat/completions", &chat_history(spaced)).await,
        200
    );
    assert_eq!(
        post_json(&url, "/v1/responses", &responses_history(spaced)).await,
        200
    );

    let seen = seen.lock().unwrap();
    assert_eq!(arguments(&seen[0]), vec![spaced.to_string()]);
    assert_eq!(arguments(&seen[1]), vec![spaced.to_string()]);
}

#[tokio::test]
async fn nothing_but_the_arguments_changes() {
    let (url, seen, _dir) = gateway().await;
    let mut body = chat_history(CUT);
    body["temperature"] = json!(0.25);
    body["messages"][0]["content"] = json!("{\"looks\": broken");
    assert_eq!(post_json(&url, "/v1/chat/completions", &body).await, 200);

    let seen = seen.lock().unwrap();
    let sent = &seen[0];
    assert_eq!(sent["temperature"], 0.25);
    assert_eq!(sent["messages"][0]["content"], "{\"looks\": broken");
    assert_eq!(sent["messages"][2]["content"], "ok");
    assert_eq!(sent["messages"][3]["content"], "go on");
}
