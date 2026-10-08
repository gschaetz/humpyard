//! Endpoint health end to end: a failing endpoint is skipped after the threshold, probed after the
//! cooldown, never causes a denial of service, and is reported by `/v1/health`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU16, AtomicUsize, Ordering};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use common::{chat_completion, chat_request, serve, usage};
use humpyard::clock::Clock;
use humpyard::config::Config;
use humpyard::server::{self, Options};
use serde_json::{Value, json};

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// An upstream whose answer can be switched while the test runs: 200, or a given error status.
struct Upstream {
    status: AtomicU16,
    calls: AtomicUsize,
}

async fn completions(State(up): State<Arc<Upstream>>) -> Response {
    up.calls.fetch_add(1, Ordering::SeqCst);
    match up.status.load(Ordering::SeqCst) {
        200 => axum::Json(chat_completion("m", "hi", Some(usage(10, 5)))).into_response(),
        code => (
            StatusCode::from_u16(code).unwrap(),
            axum::Json(json!({"error": {"message": format!("status {code}")}})),
        )
            .into_response(),
    }
}

async fn upstream(status: u16) -> (Arc<Upstream>, String) {
    let up = Arc::new(Upstream {
        status: AtomicU16::new(status),
        calls: AtomicUsize::new(0),
    });
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(up.clone());
    (up, format!("http://{}", serve(app).await))
}

struct Setup {
    url: String,
    clock: Arc<TestClock>,
    first: Arc<Upstream>,
    second: Arc<Upstream>,
}

/// Target `t`: endpoint `first` then endpoint `second`; threshold 2, cooldown 10 s, max 40 s.
async fn setup(first_status: u16, second_status: u16, health: &str) -> Setup {
    let (first, first_url) = upstream(first_status).await;
    let (second, second_url) = upstream(second_status).await;
    let toml = format!(
        r#"listen = "127.0.0.1:0"
{health}
[providers.first]
base_url = "{first_url}"
api_key_env = "K"
max_retries = 0
[providers.second]
base_url = "{second_url}"
api_key_env = "K"
max_retries = 0
[targets.t]
endpoints = [{{ provider = "first", model = "a" }}, {{ provider = "second", model = "b" }}]
"#
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    let clock = Arc::new(TestClock(AtomicI64::new(1_791_000_000_000)));
    let router = server::router_with_options(
        config,
        Options {
            clock: Some(clock.clone()),
            ..Options::default()
        },
    )
    .await
    .unwrap();
    Setup {
        url: format!("http://{}", serve(router).await),
        clock,
        first,
        second,
    }
}

const HEALTH: &str =
    "[health]\nfailure_threshold = 2\ncooldown_secs = 10\nmax_cooldown_secs = 40\n";

impl Setup {
    async fn chat(&self) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .json(&chat_request("t", false))
            .send()
            .await
            .unwrap()
    }

    async fn served_by(&self) -> String {
        let resp = self.chat().await;
        assert_eq!(resp.status(), 200);
        resp.headers()["x-humpyard-provider"]
            .to_str()
            .unwrap()
            .to_string()
    }

    fn advance(&self, secs: i64) {
        self.clock.0.fetch_add(secs * 1000, Ordering::SeqCst);
    }

    async fn health(&self) -> Value {
        let resp = reqwest::get(format!("{}/v1/health", self.url))
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        resp.json().await.unwrap()
    }
}

#[tokio::test]
async fn a_failing_first_endpoint_is_skipped_once_the_threshold_is_reached() {
    let s = setup(503, 200, HEALTH).await;
    // Two requests fail over from the first endpoint and open its breaker...
    assert_eq!(s.served_by().await, "second");
    assert_eq!(s.served_by().await, "second");
    assert_eq!(s.first.calls.load(Ordering::SeqCst), 2);
    // ...after which it is not contacted at all.
    for _ in 0..5 {
        assert_eq!(s.served_by().await, "second");
    }
    assert_eq!(
        s.first.calls.load(Ordering::SeqCst),
        2,
        "skipped while cooling down"
    );
    assert_eq!(s.second.calls.load(Ordering::SeqCst), 7);
}

#[tokio::test]
async fn after_the_cooldown_one_probe_decides_whether_the_endpoint_returns() {
    let s = setup(503, 200, HEALTH).await;
    s.served_by().await;
    s.served_by().await; // open for 10 s
    s.advance(11);

    // Still dead: the probe fails, the request is served by the second endpoint, and the
    // cooldown doubles (20 s), so the first endpoint is skipped again.
    assert_eq!(s.served_by().await, "second");
    assert_eq!(s.first.calls.load(Ordering::SeqCst), 3);
    assert_eq!(s.served_by().await, "second");
    assert_eq!(s.first.calls.load(Ordering::SeqCst), 3);
    s.advance(15);
    assert_eq!(s.served_by().await, "second");
    assert_eq!(
        s.first.calls.load(Ordering::SeqCst),
        3,
        "15 s < the doubled 20 s cooldown"
    );

    // The provider recovers; once the longer cooldown ends the probe succeeds and it is back.
    s.first.status.store(200, Ordering::SeqCst);
    s.advance(6);
    assert_eq!(s.served_by().await, "first");
    assert_eq!(s.served_by().await, "first");
}

#[tokio::test]
async fn a_target_whose_endpoints_are_all_cooling_down_is_still_tried() {
    let s = setup(503, 503, HEALTH).await;
    for _ in 0..2 {
        assert_eq!(s.chat().await.status(), 502); // both fail: the last error reaches the client
    }
    let before = (
        s.first.calls.load(Ordering::SeqCst),
        s.second.calls.load(Ordering::SeqCst),
    );
    // Both breakers are open, yet requests still go out rather than failing instantly.
    assert_eq!(s.chat().await.status(), 502);
    let after = (
        s.first.calls.load(Ordering::SeqCst),
        s.second.calls.load(Ordering::SeqCst),
    );
    assert!(
        after.0 > before.0 && after.1 > before.1,
        "{before:?} -> {after:?}"
    );

    // And the moment a provider recovers it serves, even though its breaker is still open.
    s.second.status.store(200, Ordering::SeqCst);
    assert_eq!(s.served_by().await, "second");
}

#[tokio::test]
async fn client_errors_do_not_count_against_an_endpoint() {
    let s = setup(400, 200, HEALTH).await;
    for _ in 0..6 {
        let resp = s.chat().await;
        assert_eq!(
            resp.status(),
            400,
            "a 400 stops the walk and reaches the client"
        );
    }
    assert_eq!(
        s.first.calls.load(Ordering::SeqCst),
        6,
        "never skipped: 400s say nothing about the endpoint's health"
    );
    let report = s.health().await;
    assert_eq!(report["targets"][0]["endpoints"][0]["state"], "healthy");
}

#[tokio::test]
async fn threshold_zero_turns_health_tracking_off() {
    let s = setup(503, 200, "[health]\nfailure_threshold = 0\n").await;
    for _ in 0..6 {
        assert_eq!(s.served_by().await, "second");
    }
    assert_eq!(s.first.calls.load(Ordering::SeqCst), 6, "tried every time");
}

#[tokio::test]
async fn the_health_endpoint_reports_each_endpoints_state() {
    let s = setup(503, 200, HEALTH).await;
    let report = s.health().await;
    let endpoints = &report["targets"][0]["endpoints"];
    assert_eq!(report["targets"][0]["target"], "t");
    assert_eq!(endpoints[0]["provider"], "first");
    assert_eq!(endpoints[0]["model"], "a");
    assert_eq!(endpoints[0]["state"], "healthy");

    s.served_by().await;
    s.served_by().await;
    let endpoints = &s.health().await["targets"][0]["endpoints"];
    assert_eq!(endpoints[0]["state"], "cooling_down");
    assert_eq!(endpoints[0]["consecutive_failures"], 2);
    assert_eq!(endpoints[0]["cooldown_remaining_ms"], 10_000);
    assert_eq!(endpoints[1]["state"], "healthy");

    s.advance(11);
    assert_eq!(
        s.health().await["targets"][0]["endpoints"][0]["state"],
        "probing"
    );
}

#[tokio::test]
async fn a_probe_that_gets_a_client_error_does_not_strand_the_endpoint() {
    let s = setup(503, 200, HEALTH).await;
    s.served_by().await;
    s.served_by().await; // open
    s.advance(11);
    s.first.status.store(400, Ordering::SeqCst);
    assert_eq!(
        s.chat().await.status(),
        400,
        "the probe reached the endpoint"
    );
    let probes = s.first.calls.load(Ordering::SeqCst);
    // The 400 gave no verdict, so the slot was released: the next request probes again.
    s.first.status.store(200, Ordering::SeqCst);
    assert_eq!(s.served_by().await, "first");
    assert_eq!(s.first.calls.load(Ordering::SeqCst), probes + 1);
}
