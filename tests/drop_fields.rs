//! A provider that rejects fields it does not know (Google AI rejects `store` and `prompt_cache_key`) gets them
//! removed through its `drop_fields`; other providers still receive them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
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

async fn failing(
    State(seen): State<Seen>,
    axum::Json(body): axum::Json<Value>,
) -> impl IntoResponse {
    seen.lock().unwrap().push(body);
    (StatusCode::INTERNAL_SERVER_ERROR, "boom")
}

async fn upstream(handler: axum::routing::MethodRouter<Seen>) -> (String, Seen) {
    let seen: Seen = Arc::default();
    let app = Router::new()
        .route("/chat/completions", handler)
        .with_state(seen.clone());
    (format!("http://{}", serve(app).await), seen)
}

/// A gateway with a strict provider (`drop_fields`) and a lax one, one target for each and one for the failover.
async fn gateway(strict: &str, lax: &str) -> String {
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[providers.strict]
base_url = "{strict}"
api_key_env = "K"
drop_fields = ["store", "prompt_cache_key"]
[providers.lax]
base_url = "{lax}"
api_key_env = "K"
[targets.strict]
endpoints = [{{ provider = "strict", model = "m" }}]
[targets.lax]
endpoints = [{{ provider = "lax", model = "m" }}]
[targets.both]
endpoints = [{{ provider = "strict", model = "m" }}, {{ provider = "lax", model = "m" }}]
"#
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    )
}

fn responses_request(model: &str) -> Value {
    json!({"model": model, "prompt_cache_key": "session-1", "store": false,
           "input": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]}]})
}

fn chat_request_with_cache_fields(model: &str) -> Value {
    json!({"model": model, "prompt_cache_key": "session-1", "store": false,
           "messages": [{"role": "user", "content": "hi"}]})
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

fn last(seen: &Seen) -> Value {
    seen.lock().unwrap().last().unwrap().clone()
}

#[tokio::test]
async fn a_strict_provider_is_sent_neither_field_and_a_lax_one_still_gets_the_cache_key() {
    let (strict_url, strict) = upstream(post(completions)).await;
    let (lax_url, lax) = upstream(post(completions)).await;
    let url = gateway(&strict_url, &lax_url).await;

    post_json(&url, "/v1/responses", &responses_request("strict")).await;
    let sent = last(&strict);
    assert!(sent.get("prompt_cache_key").is_none(), "{sent}");
    assert!(sent.get("store").is_none(), "{sent}");
    assert_eq!(
        sent["messages"][0]["content"], "hi",
        "the rest is untouched"
    );

    post_json(&url, "/v1/responses", &responses_request("lax")).await;
    let sent = last(&lax);
    assert_eq!(sent["prompt_cache_key"], "session-1", "{sent}");
}

#[tokio::test]
async fn a_chat_client_is_covered_too() {
    let (strict_url, strict) = upstream(post(completions)).await;
    let (lax_url, lax) = upstream(post(completions)).await;
    let url = gateway(&strict_url, &lax_url).await;

    post_json(
        &url,
        "/v1/chat/completions",
        &chat_request_with_cache_fields("strict"),
    )
    .await;
    let sent = last(&strict);
    assert!(sent.get("prompt_cache_key").is_none(), "{sent}");
    assert!(sent.get("store").is_none(), "{sent}");

    post_json(
        &url,
        "/v1/chat/completions",
        &chat_request_with_cache_fields("lax"),
    )
    .await;
    assert_eq!(last(&lax)["prompt_cache_key"], "session-1");
}

#[tokio::test]
async fn failover_gives_each_provider_its_own_list() {
    let (strict_url, strict) = upstream(post(failing)).await;
    let (lax_url, lax) = upstream(post(completions)).await;
    let url = gateway(&strict_url, &lax_url).await;

    post_json(&url, "/v1/responses", &responses_request("both")).await;
    let tried = last(&strict);
    assert!(tried.get("prompt_cache_key").is_none(), "{tried}");
    let served = last(&lax);
    assert_eq!(served["prompt_cache_key"], "session-1", "{served}");
}
