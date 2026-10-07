//! Virtual-key authentication on the inference endpoints.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use axum::Router;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde_json::{Value, json};
use switchyard_conductor::auth::generate_key;
use switchyard_conductor::config::Config;
use switchyard_conductor::server;

#[derive(Default)]
struct Mock {
    calls: AtomicUsize,
}

async fn completions(
    State(mock): State<Arc<Mock>>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    mock.calls.fetch_add(1, Ordering::SeqCst);
    let model = body["model"].as_str().unwrap().to_string();
    axum::Json(json!({"id": "c", "object": "chat.completion", "created": 1, "model": model,
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}]}))
    .into_response()
}

async fn serve(app: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

struct Harness {
    url: String,
    mock: Arc<Mock>,
    alice: String,
    bob: String,
    alice_hash: String,
}

impl Harness {
    fn upstream_calls(&self) -> usize {
        self.mock.calls.load(Ordering::SeqCst)
    }
}

async fn harness(with_keys: bool) -> Harness {
    logs();
    let mock = Arc::new(Mock::default());
    let upstream = serve(
        Router::new()
            .route("/chat/completions", post(completions))
            .with_state(mock.clone()),
    )
    .await;
    let (alice, alice_hash) = generate_key().unwrap();
    let (bob, bob_hash) = generate_key().unwrap();
    let keys = if with_keys {
        format!(
            "[keys.alice]\nsha256 = \"{alice_hash}\"\n[keys.bob]\nsha256 = \"{bob_hash}\"\nallowed_routes = [\"fast\"]\n"
        )
    } else {
        String::new()
    };
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[providers.mock]
base_url = "http://{upstream}"
api_key_env = "K"
max_retries = 0
[targets.fast]
endpoints = [{{ provider = "mock", model = "fast-m" }}]
[targets.smart]
endpoints = [{{ provider = "mock", model = "smart-m" }}]
{keys}"#
    );
    let config = Config::from_toml(&toml, |_| Some("key".into())).unwrap();
    let gateway = serve(server::router(config).await.unwrap()).await;
    Harness {
        url: format!("http://{gateway}"),
        mock,
        alice,
        bob,
        alice_hash,
    }
}

fn chat(model: &str) -> Value {
    json!({"model": model, "messages": [{"role": "user", "content": "hi"}]})
}

async fn post_with(
    h: &Harness,
    path: &str,
    body: Value,
    header: Option<(&str, String)>,
) -> reqwest::Response {
    let mut req = reqwest::Client::new()
        .post(format!("{}{path}", h.url))
        .json(&body);
    if let Some((name, value)) = header {
        req = req.header(name, value);
    }
    req.send().await.unwrap()
}

fn bearer(key: &str) -> Option<(&'static str, String)> {
    Some(("authorization", format!("Bearer {key}")))
}

#[tokio::test]
async fn missing_key_is_401_in_every_endpoint_shape_and_makes_no_upstream_call() {
    let h = harness(true).await;
    let resp = post_with(&h, "/v1/chat/completions", chat("fast"), None).await;
    assert_eq!(resp.status(), 401);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["type"], "authentication_error");

    let resp = post_with(
        &h,
        "/v1/messages",
        json!({"model": "fast", "max_tokens": 5, "messages": [{"role": "user", "content": "hi"}]}),
        None,
    )
    .await;
    assert_eq!(resp.status(), 401);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["type"], "error");
    assert_eq!(body["error"]["type"], "authentication_error");

    let resp = post_with(
        &h,
        "/v1/responses",
        json!({"model": "fast", "input": "hi"}),
        None,
    )
    .await;
    assert_eq!(resp.status(), 401);
    assert_eq!(h.upstream_calls(), 0);
}

#[tokio::test]
async fn invalid_key_is_401() {
    let h = harness(true).await;
    let resp = post_with(
        &h,
        "/v1/chat/completions",
        chat("fast"),
        bearer("sk-conductor-nope"),
    )
    .await;
    assert_eq!(resp.status(), 401);
    assert_eq!(h.upstream_calls(), 0);
}

#[tokio::test]
async fn bearer_and_x_api_key_both_authenticate() {
    let h = harness(true).await;
    let resp = post_with(&h, "/v1/chat/completions", chat("fast"), bearer(&h.alice)).await;
    assert_eq!(resp.status(), 200);
    let anthropic =
        json!({"model": "fast", "max_tokens": 5, "messages": [{"role": "user", "content": "hi"}]});
    let resp = post_with(
        &h,
        "/v1/messages",
        anthropic,
        Some(("x-api-key", h.alice.clone())),
    )
    .await;
    assert_eq!(resp.status(), 200);
    assert_eq!(h.upstream_calls(), 2);
}

#[tokio::test]
async fn open_mode_serves_requests_without_credentials() {
    let h = harness(false).await;
    assert_eq!(
        post_with(&h, "/v1/chat/completions", chat("fast"), None)
            .await
            .status(),
        200
    );
}

#[tokio::test]
async fn allowlist_blocks_other_routes_with_403() {
    let h = harness(true).await;
    let resp = post_with(&h, "/v1/chat/completions", chat("smart"), bearer(&h.bob)).await;
    assert_eq!(resp.status(), 403);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["type"], "permission_error");
    assert_eq!(h.upstream_calls(), 0);
    assert_eq!(
        post_with(&h, "/v1/chat/completions", chat("fast"), bearer(&h.bob))
            .await
            .status(),
        200
    );
}

#[tokio::test]
async fn models_list_needs_a_key_and_respects_the_allowlist() {
    let h = harness(true).await;
    let client = reqwest::Client::new();
    let anon = client
        .get(format!("{}/v1/models", h.url))
        .send()
        .await
        .unwrap();
    assert_eq!(anon.status(), 401);
    let bob: Value = client
        .get(format!("{}/v1/models", h.url))
        .bearer_auth(&h.bob)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ids: Vec<&str> = bob["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["fast"]);
}

#[derive(Clone, Default)]
struct LogBuf(Arc<Mutex<Vec<u8>>>);
impl std::io::Write for LogBuf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogBuf {
    type Writer = LogBuf;
    fn make_writer(&'a self) -> LogBuf {
        self.clone()
    }
}

/// One process-wide subscriber: per-test subscribers race on tracing's callsite cache.
fn logs() -> &'static LogBuf {
    static LOGS: OnceLock<LogBuf> = OnceLock::new();
    LOGS.get_or_init(|| {
        let buf = LogBuf::default();
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(buf.clone())
            .init();
        buf
    })
}

#[tokio::test]
async fn logs_name_the_key_id_but_never_the_key_or_its_hash() {
    let h = harness(true).await;
    post_with(&h, "/v1/chat/completions", chat("fast"), bearer(&h.alice)).await;
    post_with(
        &h,
        "/v1/chat/completions",
        chat("fast"),
        bearer("sk-conductor-bad-attempt"),
    )
    .await;
    let captured = String::from_utf8(logs().0.lock().unwrap().clone()).unwrap();
    assert!(
        captured.contains("key=alice") || captured.contains("key=\"alice\""),
        "{captured}"
    );
    assert!(!captured.contains(&h.alice), "key leaked into logs");
    assert!(
        !captured.contains("sk-conductor-bad-attempt"),
        "rejected key leaked into logs"
    );
    let hash_hex = h.alice_hash.trim_start_matches("sha256:");
    assert!(!captured.contains(hash_hex), "hash leaked into logs");
}
