#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::format_push_string,
    clippy::needless_pass_by_value
)] // test scaffolding: fail loudly, favor readability
//! `POST /v1/messages/count_tokens`: a local estimate with no upstream call and no spend.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::post;
use common::{chat_completion, chat_request, serve, usage};
use humpyard::auth::generate_key;
use humpyard::config::Config;
use humpyard::ledger::Ledger;
use humpyard::server;
use serde_json::{Value, json};

async fn completions(State(calls): State<Arc<AtomicUsize>>) -> axum::response::Response {
    calls.fetch_add(1, Ordering::SeqCst);
    axum::Json(chat_completion("fast-m", "ok", Some(usage(10, 5)))).into_response()
}

struct Harness {
    url: String,
    upstream_calls: Arc<AtomicUsize>,
    db: std::path::PathBuf,
    alice: String,
    bob: String,
    tight: String,
    _dir: tempfile::TempDir,
}

async fn harness() -> Harness {
    let calls = Arc::new(AtomicUsize::new(0));
    let upstream = serve(
        Router::new()
            .route("/chat/completions", post(completions))
            .with_state(calls.clone()),
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ledger.db");
    let keys: Vec<(String, String)> = (0..3).map(|_| generate_key().unwrap()).collect();
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[ledger]
path = "{}"
[keys.alice]
sha256 = "{}"
[keys.bob]
sha256 = "{}"
allowed_routes = ["fast"]
[keys.tight]
sha256 = "{}"
daily_tokens = 15
[providers.mock]
base_url = "http://{upstream}"
api_key_env = "K"
max_retries = 0
[targets.fast]
endpoints = [{{ provider = "mock", model = "fast-m" }}]
[targets.smart]
endpoints = [{{ provider = "mock", model = "smart-m" }}]
"#,
        db.display(),
        keys[0].1,
        keys[1].1,
        keys[2].1
    );
    let config = Config::from_toml(&toml, |_| Some("key".into())).unwrap();
    let addr = serve(server::router(config).await.unwrap()).await;
    Harness {
        url: format!("http://{addr}"),
        upstream_calls: calls,
        db,
        alice: keys[0].0.clone(),
        bob: keys[1].0.clone(),
        tight: keys[2].0.clone(),
        _dir: dir,
    }
}

impl Harness {
    async fn count(&self, key: Option<&str>, body: Value) -> reqwest::Response {
        let mut req = reqwest::Client::new()
            .post(format!("{}/v1/messages/count_tokens", self.url))
            .json(&body);
        if let Some(key) = key {
            req = req.header("x-api-key", key);
        }
        req.send().await.unwrap()
    }

    async fn tokens(&self, key: &str, body: Value) -> u64 {
        let resp = self.count(Some(key), body).await;
        assert_eq!(resp.status(), 200);
        resp.json::<Value>().await.unwrap()["input_tokens"]
            .as_u64()
            .unwrap()
    }
}

fn anthropic(model: &str, text: &str) -> Value {
    // No `max_tokens`: Anthropic's count endpoint does not take one.
    json!({"model": model, "messages": [{"role": "user", "content": text}]})
}

#[tokio::test]
async fn counts_a_conversation_without_max_tokens_and_marks_it_an_estimate() {
    let h = harness().await;
    let resp = h
        .count(
            Some(&h.alice),
            anthropic("fast", "hello world, how are you today?"),
        )
        .await;
    assert_eq!(resp.status(), 200);
    assert_eq!(resp.headers()["x-humpyard-token-count"], "estimate");
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body.as_object().unwrap().len(),
        1,
        "exactly Anthropic's shape: {body}"
    );
    assert!(body["input_tokens"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn more_messages_system_prompts_and_tools_count_more() {
    let h = harness().await;
    let base = h.tokens(&h.alice, anthropic("fast", "hi")).await;

    let mut longer = anthropic("fast", "hi");
    longer["messages"].as_array_mut().unwrap().extend([
        json!({"role": "assistant", "content": "Hello! How can I help you today?"}),
        json!({"role": "user", "content": "Tell me about hump yards."}),
    ]);
    let with_history = h.tokens(&h.alice, longer).await;
    assert!(with_history > base, "{with_history} vs {base}");

    let mut with_system = anthropic("fast", "hi");
    with_system["system"] =
        json!("You are a careful assistant that always answers in one sentence.");
    assert!(h.tokens(&h.alice, with_system).await > base);

    let mut with_tools = anthropic("fast", "hi");
    with_tools["tools"] = json!([{
        "name": "get_weather", "description": "Get the weather for a city",
        "input_schema": {"type": "object", "properties": {"city": {"type": "string"}}, "required": ["city"]}
    }]);
    assert!(h.tokens(&h.alice, with_tools).await > base + 20);
}

#[tokio::test]
async fn access_rules_match_messages_and_errors_use_anthropics_shape() {
    let h = harness().await;
    // No key.
    let resp = h.count(None, anthropic("fast", "hi")).await;
    assert_eq!(resp.status(), 401);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        (body["type"].as_str(), body["error"]["type"].as_str()),
        (Some("error"), Some("authentication_error"))
    );
    // A key allowed only `fast` counting for `smart`.
    let resp = h.count(Some(&h.bob), anthropic("smart", "hi")).await;
    assert_eq!(resp.status(), 403);
    assert_eq!(resp.json::<Value>().await.unwrap()["type"], "error");
    assert_eq!(
        h.count(Some(&h.bob), anthropic("fast", "hi"))
            .await
            .status(),
        200
    );
    // Unknown model.
    let resp = h.count(Some(&h.alice), anthropic("nope", "hi")).await;
    assert_eq!(resp.status(), 404);
    assert_eq!(
        resp.json::<Value>().await.unwrap()["error"]["type"],
        "not_found_error"
    );
    // Not JSON.
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/messages/count_tokens", h.url))
        .header("x-api-key", &h.alice)
        .body("{nope")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    assert_eq!(resp.json::<Value>().await.unwrap()["type"], "error");
}

#[tokio::test]
async fn counting_calls_no_provider_writes_no_ledger_entry_and_ignores_an_exhausted_budget() {
    let h = harness().await;
    // Spend the `tight` key's whole 15-token budget with one real call.
    let spent = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", h.url))
        .bearer_auth(&h.tight)
        .json(&chat_request("fast", false))
        .send()
        .await
        .unwrap();
    assert_eq!(spent.status(), 200);
    let blocked = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", h.url))
        .bearer_auth(&h.tight)
        .json(&chat_request("fast", false))
        .send()
        .await
        .unwrap();
    assert_eq!(blocked.status(), 402, "the key really is out of budget");

    let calls_before = h.upstream_calls.load(Ordering::SeqCst);
    for _ in 0..3 {
        assert_eq!(
            h.count(Some(&h.tight), anthropic("fast", "still counting"))
                .await
                .status(),
            200
        );
    }
    assert_eq!(
        h.upstream_calls.load(Ordering::SeqCst),
        calls_before,
        "counting must not call a provider"
    );

    // Give the writer time, then confirm the ledger holds only the one real call.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let entries = Ledger::open(&h.db)
        .await
        .unwrap()
        .entries_since(0)
        .await
        .unwrap();
    assert_eq!(entries.len(), 1, "{entries:?}");
}
