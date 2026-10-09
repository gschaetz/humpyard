//! Which upstream failures make a route fall through to its next target, and which stop at once.
//! This pins the LiteLLM-style fallback behavior (private -> public, public -> paid on context
//! overflow) that deployments rely on, including the cases that do NOT fall back.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use common::{chat_completion, chat_request, serve, usage};
use humpyard::config::Config;
use humpyard::server;
use serde_json::{Value, json};

/// How the first target's upstream fails.
#[derive(Clone)]
enum Failure {
    Status(u16, Value),
    /// Answers after this long (the provider timeout is shorter).
    Stall,
}

struct Upstream {
    failure: Option<Failure>,
    calls: AtomicUsize,
}

async fn completions(State(up): State<Arc<Upstream>>) -> Response {
    up.calls.fetch_add(1, Ordering::SeqCst);
    match &up.failure {
        None => axum::Json(chat_completion("m", "ok", Some(usage(10, 5)))).into_response(),
        Some(Failure::Status(code, body)) => (
            StatusCode::from_u16(*code).unwrap(),
            axum::Json(body.clone()),
        )
            .into_response(),
        Some(Failure::Stall) => {
            tokio::time::sleep(Duration::from_secs(5)).await;
            StatusCode::OK.into_response()
        }
    }
}

async fn upstream(failure: Option<Failure>) -> (Arc<Upstream>, String) {
    let up = Arc::new(Upstream {
        failure,
        calls: AtomicUsize::new(0),
    });
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(up.clone());
    (up, format!("http://{}", serve(app).await))
}

/// Route `r` = target `first` then `second`. Returns the response and how often each was called.
async fn run(failure: Failure) -> (u16, Option<String>, Value, usize, usize) {
    let (first, first_url) = upstream(Some(failure)).await;
    let (second, second_url) = upstream(None).await;
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[providers.p1]
base_url = "{first_url}"
api_key_env = "K"
max_retries = 0
timeout_secs = 1
[providers.p2]
base_url = "{second_url}"
api_key_env = "K"
max_retries = 0
[targets.first]
endpoints = [{{ provider = "p1", model = "a" }}]
[targets.second]
endpoints = [{{ provider = "p2", model = "b" }}]
[routes.r]
type = "passthrough"
targets = ["first", "second"]
"#
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    let resp = reqwest::Client::new()
        .post(format!("{url}/v1/chat/completions"))
        .json(&chat_request("r", false))
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let target = resp
        .headers()
        .get("x-humpyard-target")
        .map(|v| v.to_str().unwrap().to_string());
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    (
        status,
        target,
        body,
        first.calls.load(Ordering::SeqCst),
        second.calls.load(Ordering::SeqCst),
    )
}

fn err(code: &str) -> Value {
    json!({"error": {"message": "nope", "code": code}})
}

/// The request was served by the second target after trying the first.
async fn falls_back(failure: Failure) {
    let (status, target, body, first, second) = run(failure).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(target.as_deref(), Some("second"));
    assert_eq!((first, second), (1, 1));
}

/// The error reached the client and the second target was never contacted.
async fn stops(failure: Failure, expected: u16) {
    let (status, target, body, first, second) = run(failure).await;
    assert_eq!(status, expected, "{body}");
    assert_eq!(target, None);
    assert_eq!((first, second), (1, 0), "second target must not be tried");
}

#[tokio::test]
async fn a_context_window_overflow_falls_back_to_the_next_target() {
    falls_back(Failure::Status(400, err("context_length_exceeded"))).await;
}

#[tokio::test]
async fn rate_limits_server_errors_and_forbidden_fall_back() {
    falls_back(Failure::Status(429, err("rate_limit_exceeded"))).await;
    falls_back(Failure::Status(500, err("server_error"))).await;
    falls_back(Failure::Status(503, err("overloaded"))).await;
    falls_back(Failure::Status(403, err("forbidden"))).await;
}

#[tokio::test]
async fn other_client_errors_do_not_fall_back() {
    stops(Failure::Status(400, err("invalid_request")), 400).await;
    stops(Failure::Status(401, err("invalid_api_key")), 401).await;
    stops(Failure::Status(404, err("model_not_found")), 404).await;
}

#[tokio::test]
async fn a_timed_out_target_falls_back_to_the_next_one() {
    falls_back(Failure::Stall).await;
}

#[tokio::test]
async fn a_timeout_with_nothing_to_fall_back_to_is_still_a_gateway_timeout() {
    let (first, first_url) = upstream(Some(Failure::Stall)).await;
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[providers.p1]
base_url = "{first_url}"
api_key_env = "K"
max_retries = 0
timeout_secs = 1
[targets.only]
endpoints = [{{ provider = "p1", model = "a" }}]
"#
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    let resp = reqwest::Client::new()
        .post(format!("{url}/v1/chat/completions"))
        .json(&chat_request("only", false))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 504);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["message"], "upstream timed out", "{body}");
    assert_eq!(first.calls.load(Ordering::SeqCst), 1);
}
