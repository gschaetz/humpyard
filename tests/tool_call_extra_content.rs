//! A provider that wants provider data echoed on replayed tool calls (Gemini 3.x wants a thought signature) gets the
//! configured `extra_content` on every assistant tool call; other providers and requests without tool calls are untouched.
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

type Seen = Arc<Mutex<Vec<Value>>>;

async fn completions(
    State(seen): State<Seen>,
    axum::Json(body): axum::Json<Value>,
) -> impl IntoResponse {
    seen.lock().unwrap().push(body);
    axum::Json(chat_completion("m", "ok", Some(usage(10, 5))))
}

async fn upstream() -> (String, Seen) {
    let seen: Seen = Arc::default();
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(seen.clone());
    (format!("http://{}", serve(app).await), seen)
}

const SIGNATURE: &str = "skip_thought_signature_validator";

async fn gateway(gem: &str, lax: &str) -> String {
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[providers.gem]
base_url = "{gem}"
api_key_env = "K"
drop_fields = ["store", "prompt_cache_key"]
tool_call_extra_content = {{ google = {{ thought_signature = "{SIGNATURE}" }} }}
[providers.lax]
base_url = "{lax}"
api_key_env = "K"
[targets.gem]
endpoints = [{{ provider = "gem", model = "m" }}]
[targets.lax]
endpoints = [{{ provider = "lax", model = "m" }}]
"#
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    )
}

fn replay_responses(model: &str) -> Value {
    json!({"model": model, "prompt_cache_key": "s1", "store": false, "input": [
        {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]},
        {"type": "function_call", "call_id": "c1", "name": "shell", "arguments": "{\"cmd\":\"ls\"}"},
        {"type": "function_call_output", "call_id": "c1", "output": "a.txt"}
    ]})
}

fn replay_chat(model: &str) -> Value {
    json!({"model": model, "messages": [
        {"role": "user", "content": "hi"},
        {"role": "assistant", "content": null, "tool_calls": [
            {"id": "c1", "type": "function", "function": {"name": "shell", "arguments": "{\"cmd\":\"ls\"}"}}]},
        {"role": "tool", "tool_call_id": "c1", "content": "a.txt"}
    ]})
}

async fn post_json(url: &str, path: &str, body: &Value) {
    let resp = reqwest::Client::new()
        .post(format!("{url}{path}"))
        .json(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200, "{:?}", resp.text().await);
}

fn tool_call(seen: &Seen) -> Value {
    let body = seen.lock().unwrap().last().unwrap().clone();
    body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|m| m["tool_calls"].get(0).cloned())
        .unwrap_or_else(|| panic!("no assistant tool call in {body}"))
}

#[tokio::test]
async fn a_replayed_tool_call_carries_the_providers_extra_content() {
    let (gem_url, gem) = upstream().await;
    let (lax_url, lax) = upstream().await;
    let url = gateway(&gem_url, &lax_url).await;

    post_json(&url, "/v1/responses", &replay_responses("gem")).await;
    let call = tool_call(&gem);
    assert_eq!(
        call["extra_content"]["google"]["thought_signature"], SIGNATURE,
        "{call}"
    );
    assert_eq!(call["id"], "c1");
    assert_eq!(call["function"]["name"], "shell");
    let mut gem_body = gem.lock().unwrap().last().unwrap().clone();
    assert!(
        gem_body.get("prompt_cache_key").is_none(),
        "drop_fields still applies: {gem_body}"
    );

    post_json(&url, "/v1/responses", &replay_responses("lax")).await;
    let call = tool_call(&lax);
    assert!(call.get("extra_content").is_none(), "{call}");

    // Apart from the added content and the dropped fields, the two bodies are the same.
    let mut lax_body = lax.lock().unwrap().last().unwrap().clone();
    gem_body["messages"][1]["tool_calls"][0]
        .as_object_mut()
        .unwrap()
        .remove("extra_content");
    lax_body.as_object_mut().unwrap().remove("prompt_cache_key");
    lax_body.as_object_mut().unwrap().remove("store");
    assert_eq!(gem_body, lax_body);
}

#[tokio::test]
async fn a_chat_client_replay_is_covered_too() {
    let (gem_url, gem) = upstream().await;
    let (lax_url, _lax) = upstream().await;
    let url = gateway(&gem_url, &lax_url).await;

    post_json(&url, "/v1/chat/completions", &replay_chat("gem")).await;
    let call = tool_call(&gem);
    assert_eq!(
        call["extra_content"]["google"]["thought_signature"], SIGNATURE,
        "{call}"
    );
}

#[tokio::test]
async fn a_request_without_tool_calls_is_sent_as_it_would_be_without_the_setting() {
    let (gem_url, gem) = upstream().await;
    let (lax_url, lax) = upstream().await;
    let url = gateway(&gem_url, &lax_url).await;
    let plain = |m: &str| json!({"model": m, "messages": [{"role": "user", "content": "hi"}]});

    post_json(&url, "/v1/chat/completions", &plain("gem")).await;
    post_json(&url, "/v1/chat/completions", &plain("lax")).await;
    let sent = gem.lock().unwrap().last().unwrap().clone();
    let reference = lax.lock().unwrap().last().unwrap().clone();
    assert_eq!(sent, reference);
}
