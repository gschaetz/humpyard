//! `humpyard_endpoint_unavailable_total` (ADR 0023): an attempt the endpoint answers with "I cannot serve
//! this at all" (404, 410, 401, a 400 naming a missing model) is counted by provider and target, once; a
//! rate limit, a timeout or a server error is not.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use axum::Router;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use common::{chat_completion, chat_request, serve, usage};
use humpyard::config::Config;
use humpyard::server;
use serde_json::{Value, json};

struct Rig {
    url: String,
    credential: String,
}

async fn failing(status: u16, body: Value) -> String {
    let app = Router::new().route(
        "/chat/completions",
        post(move || {
            let body = body.clone();
            async move { (StatusCode::from_u16(status).unwrap(), axum::Json(body)).into_response() }
        }),
    );
    format!("http://{}", serve(app).await)
}

async fn live() -> String {
    let app = Router::new().route(
        "/chat/completions",
        post(|| async { axum::Json(chat_completion("m", "ok", Some(usage(10, 5)))) }),
    );
    format!("http://{}", serve(app).await)
}

/// Target `a-dead` (provider `pd`) answers `status`; target `b-live` (provider `pl`) answers; `pool` tries
/// them in that order, so a dead first target fails over to the live one.
async fn rig(status: u16, body: Value) -> Rig {
    let dead = failing(status, body).await;
    let alive = live().await;
    let (credential, credential_hash) = humpyard::auth::generate_key().unwrap();
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[metrics]
enabled = true
bearer_sha256 = "{credential_hash}"
[providers.pd]
base_url = "{dead}"
api_key_env = "PATH"
max_retries = 0
[providers.pl]
base_url = "{alive}"
api_key_env = "PATH"
max_retries = 0
[targets.a-dead]
endpoints = [{{ provider = "pd", model = "m" }}]
[targets.b-live]
endpoints = [{{ provider = "pl", model = "m" }}]
[routes.fast]
type = "fastest"
targets = ["a-dead", "b-live"]
explore = 0.0
fallback_on = ["unavailable", "rate_limit", "server_error", "timeout"]
"#
    );
    let config = Config::from_toml(&toml, |n| std::env::var(n).ok()).unwrap();
    Rig {
        url: format!(
            "http://{}",
            serve(server::router(config).await.unwrap()).await
        ),
        credential,
    }
}

impl Rig {
    async fn ask(&self) -> u16 {
        reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .json(&chat_request("fast", false))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    }

    async fn scrape(&self) -> String {
        let resp = reqwest::Client::new()
            .get(format!("{}/metrics", self.url))
            .bearer_auth(&self.credential)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        resp.text().await.unwrap()
    }
}

const SERIES: &str = r#"humpyard_endpoint_unavailable_total{provider="pd",target="a-dead"}"#;

fn plain() -> Value {
    json!({"error": {"message": "nope"}})
}

#[tokio::test]
async fn an_answer_that_says_the_endpoint_cannot_serve_is_counted_once() {
    let gone = json!({"error": {"message": "Model m has been deprecated. Use n instead"}});
    let missing = json!({"error": {"message": "no such model", "code": "model_not_found"}});
    for (status, body) in [(410, gone), (404, plain()), (401, plain()), (400, missing)] {
        let rig = rig(status, body).await;
        assert_eq!(rig.ask().await, 200, "status {status} fails over");
        let text = rig.scrape().await;
        assert!(
            text.contains(&format!("{SERIES} 1")),
            "status {status} should count once:\n{text}"
        );
    }
}

#[tokio::test]
async fn failures_of_other_classes_are_not_counted() {
    for status in [429, 500, 503] {
        let rig = rig(status, plain()).await;
        assert_eq!(rig.ask().await, 200, "status {status} fails over");
        let text = rig.scrape().await;
        assert!(
            !text.contains("humpyard_endpoint_unavailable_total{"),
            "status {status} must not raise the counter:\n{text}"
        );
    }
}

#[tokio::test]
async fn the_counter_has_only_configured_names_and_no_error_text() {
    let gone = json!({"error": {"message": "evil\"model{route=\"x\"} is gone"}});
    let rig = rig(410, gone).await;
    assert_eq!(rig.ask().await, 200);
    let text = rig.scrape().await;
    assert!(text.contains(&format!("{SERIES} 1")), "{text}");
    assert!(!text.contains("evil"), "error text reached labels:\n{text}");
}
