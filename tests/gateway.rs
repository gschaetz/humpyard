//! End-to-end tests: the real gateway router in front of a mock OpenAI-compatible upstream.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::format_collect,
    clippy::needless_pass_by_value
)] // test scaffolding: fail loudly, favor readability

use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use humpyard::config::Config;
use humpyard::server;
use serde_json::{Value, json};

const KEY: &str = "sk-test-secret-key";

#[derive(Default)]
struct Mock {
    auth: Mutex<Vec<String>>,
    stream_dropped: AtomicBool,
}

struct DropFlag(Arc<Mock>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.stream_dropped.store(true, Ordering::SeqCst);
    }
}

fn chunk(delta: Value, finish: Option<&str>) -> Value {
    json!({"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1, "model": "m",
           "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]})
}

async fn completions(
    State(mock): State<Arc<Mock>>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    mock.auth
        .lock()
        .unwrap()
        .push(headers["authorization"].to_str().unwrap().to_string());
    let model = body["model"].as_str().unwrap().to_string();
    if model == "limited" {
        return (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            axum::Json(json!({"error": {"message": "slow down"}})),
        )
            .into_response();
    }
    if body["stream"] == true {
        let guard = DropFlag(mock.clone());
        let slow = model == "slow";
        let events = async_stream::stream! {
            let _guard = guard;
            yield Ok::<_, Infallible>(Event::default().data(chunk(json!({"role": "assistant", "content": "Hel"}), None).to_string()));
            if slow {
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
            yield Ok(Event::default().data(chunk(json!({"content": "lo"}), None).to_string()));
            yield Ok(Event::default().data(chunk(json!({}), Some("stop")).to_string()));
            yield Ok(Event::default().data("[DONE]"));
        };
        return Sse::new(events).into_response();
    }
    let message = if model == "tools" {
        json!({"role": "assistant", "content": null, "tool_calls": [{"id": "call_1", "type": "function",
               "function": {"name": "get_weather", "arguments": "{\"city\":\"Paris\"}"}}]})
    } else {
        json!({"role": "assistant", "content": "Hello"})
    };
    let finish = if model == "tools" {
        "tool_calls"
    } else {
        "stop"
    };
    axum::Json(
        json!({"id": "chatcmpl-1", "object": "chat.completion", "created": 1, "model": model,
        "choices": [{"index": 0, "message": message, "finish_reason": finish}],
        "usage": {"prompt_tokens": 3, "completion_tokens": 2, "total_tokens": 5}}),
    )
    .into_response()
}

async fn serve(app: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

struct Harness {
    gateway: String,
    mock: Arc<Mock>,
    client: reqwest::Client,
}

async fn harness_with_upstream(upstream: &str, mock: Arc<Mock>) -> Harness {
    logs();
    let targets: String = ["m", "limited", "slow", "tools"]
        .iter()
        .map(|t| {
            format!("[targets.{t}]\nendpoints = [{{ provider = \"mock\", model = \"{t}\" }}]\n")
        })
        .collect();
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[providers.mock]
base_url = "{upstream}"
api_key_env = "KEY"
timeout_secs = 5
{targets}"#
    );
    let config = Config::from_toml(&toml, |_| Some(KEY.to_string())).unwrap();
    let gateway = serve(server::router(config).await.unwrap()).await;
    Harness {
        gateway: format!("http://{gateway}"),
        mock,
        client: reqwest::Client::new(),
    }
}

async fn harness() -> Harness {
    let mock = Arc::new(Mock::default());
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(mock.clone());
    let upstream = serve(app).await;
    harness_with_upstream(&format!("http://{upstream}"), mock).await
}

impl Harness {
    async fn post(&self, path: &str, body: Value) -> reqwest::Response {
        self.client
            .post(format!("{}{path}", self.gateway))
            .json(&body)
            .send()
            .await
            .unwrap()
    }
}

fn chat_body(model: &str, stream: bool) -> Value {
    json!({"model": model, "stream": stream, "messages": [{"role": "user", "content": "hi"}]})
}

#[tokio::test]
async fn health_and_models() {
    let h = harness().await;
    let health = h
        .client
        .get(format!("{}/healthz", h.gateway))
        .send()
        .await
        .unwrap();
    assert_eq!(health.status(), 200);
    let models: Value = h
        .client
        .get(format!("{}/v1/models", h.gateway))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ids: Vec<&str> = models["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["limited", "m", "slow", "tools"]);
}

#[tokio::test]
async fn chat_non_streaming_sends_bearer_and_returns_openai_shape() {
    let h = harness().await;
    let resp = h.post("/v1/chat/completions", chat_body("m", false)).await;
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["choices"][0]["message"]["content"], "Hello");
    assert_eq!(h.mock.auth.lock().unwrap()[0], format!("Bearer {KEY}"));
}

#[tokio::test]
async fn chat_streaming_ends_with_done() {
    let h = harness().await;
    let text = h
        .post("/v1/chat/completions", chat_body("m", true))
        .await
        .text()
        .await
        .unwrap();
    assert!(text.contains("\"Hel\""), "{text}");
    assert!(text.trim_end().ends_with("data: [DONE]"), "{text}");
}

#[tokio::test]
async fn anthropic_client_gets_anthropic_shape() {
    let h = harness().await;
    let body =
        json!({"model": "m", "max_tokens": 50, "messages": [{"role": "user", "content": "hi"}]});
    let resp: Value = h
        .post("/v1/messages", body.clone())
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(resp["type"], "message", "{resp}");
    assert_eq!(resp["content"][0]["text"], "Hello");

    let mut streaming = body;
    streaming["stream"] = json!(true);
    let text = h
        .post("/v1/messages", streaming)
        .await
        .text()
        .await
        .unwrap();
    assert!(text.contains("event: message_start"), "{text}");
    assert!(text.contains("event: content_block_delta"), "{text}");
    assert!(text.contains("event: message_stop"), "{text}");
}

#[tokio::test]
async fn responses_client_gets_responses_shape() {
    let h = harness().await;
    let body = json!({"model": "m", "input": "hi"});
    let resp: Value = h
        .post("/v1/responses", body.clone())
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(resp["object"], "response", "{resp}");

    let mut streaming = body;
    streaming["stream"] = json!(true);
    let text = h
        .post("/v1/responses", streaming)
        .await
        .text()
        .await
        .unwrap();
    let created = text.find("event: response.created").expect(&text);
    let completed = text.find("event: response.completed").expect(&text);
    assert!(created < completed, "{text}");
}

#[tokio::test]
async fn responses_tool_call_becomes_function_call_item() {
    let h = harness().await;
    let body = json!({"model": "tools", "input": "weather?", "tools": [
        {"type": "function", "name": "get_weather", "parameters": {"type": "object", "properties": {"city": {"type": "string"}}}}]});
    let resp: Value = h.post("/v1/responses", body).await.json().await.unwrap();
    let item = resp["output"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["type"] == "function_call")
        .expect("function_call item");
    assert_eq!(item["call_id"], "call_1");
    assert_eq!(item["name"], "get_weather");
    let arguments: Value = serde_json::from_str(item["arguments"].as_str().unwrap()).unwrap();
    assert_eq!(arguments, json!({"city": "Paris"}));
}

#[tokio::test]
async fn previous_response_id_is_rejected() {
    let h = harness().await;
    let resp = h
        .post(
            "/v1/responses",
            json!({"model": "m", "input": "hi", "previous_response_id": "resp_1"}),
        )
        .await;
    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().await.unwrap();
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("previous_response_id")
    );
}

#[tokio::test]
async fn errors_use_the_endpoint_error_shape() {
    let h = harness().await;
    let bad = h
        .client
        .post(format!("{}/v1/messages", h.gateway))
        .body("{nope")
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
    let body: Value = bad.json().await.unwrap();
    assert_eq!(body["type"], "error");
    assert_eq!(body["error"]["type"], "invalid_request_error");

    let unknown = h
        .post("/v1/chat/completions", chat_body("nope", false))
        .await;
    assert_eq!(unknown.status(), 404);
    let body: Value = unknown.json().await.unwrap();
    assert_eq!(body["error"]["type"], "not_found_error");
}

#[tokio::test]
async fn upstream_rate_limit_passes_through() {
    let h = harness().await;
    let resp = h
        .post("/v1/chat/completions", chat_body("limited", false))
        .await;
    assert_eq!(resp.status(), 429);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["message"], "slow down");
}

#[tokio::test]
async fn unreachable_upstream_is_502() {
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = closed.local_addr().unwrap();
    drop(closed);
    let h = harness_with_upstream(&format!("http://{addr}"), Arc::new(Mock::default())).await;
    let resp = h.post("/v1/chat/completions", chat_body("m", false)).await;
    assert_eq!(resp.status(), 502);
}

#[tokio::test]
async fn first_chunk_arrives_before_upstream_finishes() {
    let h = harness().await;
    let mut resp = h
        .post("/v1/chat/completions", chat_body("slow", true))
        .await;
    let first = tokio::time::timeout(Duration::from_secs(3), resp.chunk())
        .await
        .expect("first chunk should not wait for the 30s upstream pause")
        .unwrap()
        .unwrap();
    assert!(String::from_utf8_lossy(&first).contains("Hel"));
}

#[tokio::test]
async fn client_disconnect_cancels_upstream() {
    let h = harness().await;
    let mut resp = h
        .post("/v1/chat/completions", chat_body("slow", true))
        .await;
    resp.chunk().await.unwrap().unwrap();
    drop(resp);
    for _ in 0..50 {
        if h.mock.stream_dropped.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("upstream stream was not dropped after the client disconnected");
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

/// One process-wide subscriber: per-test subscribers race on tracing's global callsite cache.
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
async fn api_key_never_appears_in_logs() {
    let h = harness().await;
    h.post("/v1/chat/completions", chat_body("m", false)).await;
    h.post("/v1/chat/completions", chat_body("limited", false))
        .await;
    let captured = String::from_utf8(logs().0.lock().unwrap().clone()).unwrap();
    assert!(
        captured.contains("request"),
        "expected request logs, got: {captured}"
    );
    assert!(!captured.contains(KEY), "{captured}");
}
