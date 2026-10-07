//! Provider pool behavior: endpoint order, per-endpoint model and key, failover rules.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use serde_json::{Value, json};
use switchyard_conductor::config::Config;
use switchyard_conductor::server;

#[derive(Clone, Copy)]
enum Mode {
    Ok,
    Status(u16),
    Slow,
    StreamThenDie,
}

struct Upstream {
    mode: Mode,
    calls: Mutex<Vec<(String, String)>>,
}

impl Upstream {
    fn new(mode: Mode) -> Arc<Self> {
        Arc::new(Self {
            mode,
            calls: Mutex::new(vec![]),
        })
    }
    fn calls(&self) -> Vec<(String, String)> {
        self.calls.lock().unwrap().clone()
    }
}

fn chunk(text: &str) -> String {
    json!({"id": "c", "object": "chat.completion.chunk", "created": 1, "model": "m",
           "choices": [{"index": 0, "delta": {"role": "assistant", "content": text}, "finish_reason": null}]})
    .to_string()
}

async fn completions(
    State(up): State<Arc<Upstream>>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    up.calls.lock().unwrap().push((
        body["model"].as_str().unwrap().to_string(),
        headers["authorization"].to_str().unwrap().to_string(),
    ));
    match up.mode {
        Mode::Status(code) => (
            StatusCode::from_u16(code).unwrap(),
            axum::Json(json!({"error": {"message": format!("status {code}")}})),
        )
            .into_response(),
        Mode::Slow => {
            tokio::time::sleep(Duration::from_secs(5)).await;
            StatusCode::OK.into_response()
        }
        Mode::StreamThenDie => {
            let events = async_stream::stream! {
                yield Ok::<_, std::io::Error>(Event::default().data(chunk("partial")));
                tokio::time::sleep(Duration::from_millis(50)).await;
                yield Err(std::io::Error::other("upstream died"));
            };
            Sse::new(events).into_response()
        }
        Mode::Ok => axum::Json(json!({"id": "c", "object": "chat.completion", "created": 1, "model": "m",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}]}))
        .into_response(),
    }
}

async fn serve(app: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    addr
}

async fn upstream(up: &Arc<Upstream>) -> String {
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(up.clone());
    format!("http://{}", serve(app).await)
}

/// Gateway with target `t` whose endpoints are p1 (model alpha, key k1) then p2 (model beta, key k2).
async fn gateway(p1_url: &str, p2_url: &str, timeout_secs: u64) -> String {
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[providers.p1]
base_url = "{p1_url}"
api_key_env = "K1"
max_retries = 0
timeout_secs = {timeout_secs}
[providers.p2]
base_url = "{p2_url}"
api_key_env = "K2"
max_retries = 0
timeout_secs = {timeout_secs}
[targets.t]
endpoints = [{{ provider = "p1", model = "alpha" }}, {{ provider = "p2", model = "beta" }}]
"#
    );
    let config = Config::from_toml(&toml, |name| Some(format!("key-{name}"))).unwrap();
    format!("http://{}", serve(server::router(config).unwrap()).await)
}

async fn chat(gateway: &str, stream: bool) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{gateway}/v1/chat/completions"))
        .json(&json!({"model": "t", "stream": stream, "messages": [{"role": "user", "content": "hi"}]}))
        .send()
        .await
        .unwrap()
}

fn header(resp: &reqwest::Response, name: &str) -> String {
    resp.headers()
        .get(name)
        .map(|v| v.to_str().unwrap().to_string())
        .unwrap_or_default()
}

#[tokio::test]
async fn first_endpoint_serves_with_its_own_model_and_key() {
    let (a, b) = (Upstream::new(Mode::Ok), Upstream::new(Mode::Ok));
    let g = gateway(&upstream(&a).await, &upstream(&b).await, 5).await;
    let resp = chat(&g, false).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(header(&resp, "x-conductor-provider"), "p1");
    assert_eq!(header(&resp, "x-conductor-target"), "t");
    assert_eq!(
        a.calls(),
        [("alpha".to_string(), "Bearer key-K1".to_string())]
    );
    assert!(b.calls().is_empty());
}

#[tokio::test]
async fn rate_limit_and_server_errors_fail_over_with_the_next_endpoints_model_and_key() {
    for code in [429, 500, 503] {
        let (a, b) = (Upstream::new(Mode::Status(code)), Upstream::new(Mode::Ok));
        let g = gateway(&upstream(&a).await, &upstream(&b).await, 5).await;
        let resp = chat(&g, false).await;
        assert_eq!(resp.status(), 200, "first endpoint status {code}");
        assert_eq!(header(&resp, "x-conductor-provider"), "p2");
        assert_eq!(
            b.calls(),
            [("beta".to_string(), "Bearer key-K2".to_string())]
        );
    }
}

#[tokio::test]
async fn unreachable_endpoint_fails_over() {
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    let b = Upstream::new(Mode::Ok);
    let g = gateway(&dead, &upstream(&b).await, 5).await;
    let resp = chat(&g, false).await;
    assert_eq!(resp.status(), 200);
    assert_eq!(header(&resp, "x-conductor-provider"), "p2");
}

#[tokio::test]
async fn client_errors_do_not_fail_over() {
    let (a, b) = (Upstream::new(Mode::Status(400)), Upstream::new(Mode::Ok));
    let g = gateway(&upstream(&a).await, &upstream(&b).await, 5).await;
    let resp = chat(&g, false).await;
    assert_eq!(resp.status(), 400);
    assert!(
        b.calls().is_empty(),
        "a 400 would fail identically elsewhere"
    );
}

#[tokio::test]
async fn exhausted_failover_reports_the_last_endpoints_error() {
    let (a, b) = (
        Upstream::new(Mode::Status(500)),
        Upstream::new(Mode::Status(429)),
    );
    let g = gateway(&upstream(&a).await, &upstream(&b).await, 5).await;
    let resp = chat(&g, false).await;
    assert_eq!(resp.status(), 429);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["message"], "status 429");
    assert_eq!(a.calls().len(), 1);
    assert_eq!(b.calls().len(), 1);
}

#[tokio::test]
async fn all_endpoints_unreachable_is_502() {
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    let g = gateway(&dead, &dead, 5).await;
    assert_eq!(chat(&g, false).await.status(), 502);
}

#[tokio::test]
async fn all_endpoints_timing_out_is_504() {
    let (a, b) = (Upstream::new(Mode::Slow), Upstream::new(Mode::Slow));
    let g = gateway(&upstream(&a).await, &upstream(&b).await, 1).await;
    assert_eq!(chat(&g, false).await.status(), 504);
    assert_eq!(a.calls().len(), 1);
    assert_eq!(
        b.calls().len(),
        1,
        "a timeout fails over to the next endpoint"
    );
}

#[tokio::test]
async fn no_failover_after_the_stream_has_started() {
    let (a, b) = (Upstream::new(Mode::StreamThenDie), Upstream::new(Mode::Ok));
    let g = gateway(&upstream(&a).await, &upstream(&b).await, 5).await;
    let resp = chat(&g, true).await;
    assert_eq!(resp.status(), 200);
    let text = resp.text().await.unwrap_or_default();
    assert!(text.contains("partial"), "{text}");
    assert!(
        !text.contains("[DONE]"),
        "a failed turn must not look finished: {text}"
    );
    assert!(b.calls().is_empty());
}
