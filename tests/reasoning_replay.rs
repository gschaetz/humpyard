//! Replaying a reasoning item (Codex does this on every turn after the first) must not produce a
//! request the chat-completions upstream rejects.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::post;
use common::{chat_completion, serve, usage};
use humpyard::config::Config;
use humpyard::server;
use serde_json::{Value, json};

async fn completions(
    State(seen): State<Arc<Mutex<Vec<Value>>>>,
    axum::Json(body): axum::Json<Value>,
) -> impl IntoResponse {
    seen.lock().unwrap().push(body);
    axum::Json(chat_completion("m", "ok", Some(usage(10, 5))))
}

#[tokio::test]
async fn a_replayed_reasoning_item_is_sent_upstream_in_a_form_chat_providers_accept() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(seen.clone());
    let upstream = format!("http://{}", serve(app).await);
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[providers.p]
base_url = "{upstream}"
api_key_env = "K"
[targets.t]
endpoints = [{{ provider = "p", model = "m" }}]
"#
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );

    let variants = [
        json!({"type": "reasoning", "id": "rs_1", "status": "completed",
               "summary": [{"type": "summary_text", "text": "Need to inspect the files first."}]}),
        json!({"type": "reasoning", "id": "rs_1", "summary": [{"type": "summary_text", "text": "Need to inspect."}],
               "content": null, "encrypted_content": null}),
        json!({"type": "reasoning", "id": "rs_1", "summary": [],
               "content": [{"type": "reasoning_text", "text": "raw thoughts"}]}),
        json!({"type": "reasoning", "id": "rs_1", "summary": [{"type": "summary_text", "text": "x"}],
               "encrypted_content": "gAAAAABabc"}),
    ];
    for (n, reasoning) in variants.into_iter().enumerate() {
        let body = json!({
            "model": "t",
            "input": [
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]},
                reasoning,
                {"type": "function_call", "call_id": "c1", "name": "shell", "arguments": "{\"cmd\":\"ls\"}"},
                {"type": "function_call_output", "call_id": "c1", "output": "a.txt"}
            ]
        });
        let resp = reqwest::Client::new()
            .post(format!("{url}/v1/responses"))
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "{:?}", resp.text().await);
        let upstream_body = seen.lock().unwrap().last().unwrap().clone();
        let assistant = &upstream_body["messages"][1];
        assert!(
            assistant.get("reasoning_details").is_none(),
            "variant {n}: the Responses-shaped detail must not reach a chat provider: {assistant}"
        );
        assert!(
            assistant["reasoning"]
                .as_str()
                .is_some_and(|r| !r.is_empty()),
            "variant {n}: the reasoning text is kept: {assistant}"
        );
        assert_eq!(assistant["tool_calls"][0]["id"], "c1");
    }
}

#[tokio::test]
async fn chat_style_reasoning_details_are_passed_through_untouched() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(seen.clone());
    let upstream = format!("http://{}", serve(app).await);
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[providers.p]
base_url = "{upstream}"
api_key_env = "K"
[targets.t]
endpoints = [{{ provider = "p", model = "m" }}]
"#
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    let detail = json!({"type": "reasoning.encrypted", "data": "opaque", "index": 0});
    let body = json!({"model": "t", "messages": [
        {"role": "user", "content": "hi"},
        {"role": "assistant", "content": "ok", "reasoning_details": [detail]},
        {"role": "user", "content": "go on"}
    ]});
    let resp = reqwest::Client::new()
        .post(format!("{url}/v1/chat/completions"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let upstream_body = seen.lock().unwrap().last().unwrap().clone();
    assert_eq!(
        upstream_body["messages"][1]["reasoning_details"],
        json!([detail])
    );
}

#[tokio::test]
async fn the_developer_role_is_sent_as_system() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(seen.clone());
    let upstream = format!("http://{}", serve(app).await);
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[providers.p]
base_url = "{upstream}"
api_key_env = "K"
[targets.t]
endpoints = [{{ provider = "p", model = "m" }}]
"#
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    let body = json!({"model": "t", "input": [
        {"type": "message", "role": "developer", "content": [{"type": "input_text", "text": "sandbox rules"}]},
        {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]},
        {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "hello"}]},
        {"type": "message", "role": "developer", "content": [{"type": "input_text", "text": "permissions changed"}]},
        {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "go on"}]}
    ]});
    let resp = reqwest::Client::new()
        .post(format!("{url}/v1/responses"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let upstream_body = seen.lock().unwrap().last().unwrap().clone();
    let roles: Vec<&str> = upstream_body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert!(!roles.contains(&"developer"), "{roles:?}");
    assert_eq!(
        roles.iter().filter(|r| **r == "system").count(),
        2,
        "{roles:?}"
    );
}
