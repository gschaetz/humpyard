//! Stated retry times (ADR 0018, amended): a 429 or 503 whose body says when the endpoint will serve
//! again cools it for exactly that long, at once; anything else is handled as before.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use common::{chat_completion, chat_request, serve, usage};
use humpyard::clock::{Clock, SystemClock};
use humpyard::config::Config;
use humpyard::server::{self, Options};
use serde_json::{Value, json};

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

/// A mock upstream that answers with a configurable status and error body.
struct Upstream {
    status: Mutex<(u16, String)>,
    calls: AtomicUsize,
}

impl Upstream {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
    fn answer(&self, status: u16, body: &str) {
        *self.status.lock().unwrap() = (status, body.to_string());
    }
}

async fn completions(State(up): State<Arc<Upstream>>) -> Response {
    up.calls.fetch_add(1, Ordering::SeqCst);
    let (status, body) = up.status.lock().unwrap().clone();
    if status == 200 {
        return axum::Json(chat_completion("m", "ok", Some(usage(10, 5)))).into_response();
    }
    (
        StatusCode::from_u16(status).unwrap(),
        axum::Json(json!({"error": {"message": body}})),
    )
        .into_response()
}

async fn upstream(status: u16, body: &str) -> (Arc<Upstream>, String) {
    let up = Arc::new(Upstream {
        status: Mutex::new((status, body.to_string())),
        calls: AtomicUsize::new(0),
    });
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(up.clone());
    (up, format!("http://{}", serve(app).await))
}

struct Rig {
    url: String,
    key: String,
    credential: String,
    clock: Arc<TestClock>,
    free: Arc<Upstream>,
    paid: Arc<Upstream>,
    _dir: tempfile::TempDir,
}

/// Route `duo` tries `free` (provider `pf`) then `paid` (provider `pp`). The breaker needs three
/// failures to open on its own, so a cooldown after one failure can only come from a stated time.
async fn rig(free_status: u16, free_body: &str, health_extra: &str) -> Rig {
    let (free, free_url) = upstream(free_status, free_body).await;
    let (paid, paid_url) = upstream(200, "").await;
    let dir = tempfile::tempdir().unwrap();
    let (key, key_hash) = humpyard::auth::generate_key().unwrap();
    let (credential, credential_hash) = humpyard::auth::generate_key().unwrap();
    let toml = format!(
        r#"listen = "127.0.0.1:0"
# The test clock sits in 1970; retention would prune each decision once it is written.
decision_retention_days = 0
[ledger]
path = "{db}"
[metrics]
enabled = true
bearer_sha256 = "{credential_hash}"
[health]
failure_threshold = 3
cooldown_secs = 10
max_cooldown_secs = 35
{health_extra}
[keys.alice]
sha256 = "{key_hash}"
[providers.pf]
base_url = "{free_url}"
api_key_env = "K"
max_retries = 0
[providers.pp]
base_url = "{paid_url}"
api_key_env = "K"
max_retries = 0
[targets.free]
endpoints = [{{ provider = "pf", model = "m" }}]
[targets.paid]
endpoints = [{{ provider = "pp", model = "m" }}]
[routes.duo]
type = "passthrough"
targets = ["free", "paid"]
"#,
        db = dir.path().join("ledger.db").display(),
    );
    let clock = Arc::new(TestClock(AtomicI64::new(SystemClock.now_ms())));
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    let app = server::router_with_options(
        config,
        Options {
            clock: Some(clock.clone()),
            ..Options::default()
        },
    )
    .await
    .unwrap();
    Rig {
        url: format!("http://{}", serve(app).await),
        key,
        credential,
        clock,
        free,
        paid,
        _dir: dir,
    }
}

impl Rig {
    async fn ask(&self) -> (u16, String) {
        let resp = reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .bearer_auth(&self.key)
            .json(&chat_request("duo", false))
            .send()
            .await
            .unwrap();
        let target = resp
            .headers()
            .get("x-humpyard-target")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        (resp.status().as_u16(), target)
    }

    fn advance_ms(&self, ms: i64) {
        self.clock.0.fetch_add(ms, Ordering::SeqCst);
    }

    /// The `free` target's only endpoint, as `/v1/health` reports it.
    async fn free_health(&self) -> Value {
        let body: Value = reqwest::Client::new()
            .get(format!("{}/v1/health", self.url))
            .bearer_auth(&self.key)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        body["targets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["target"] == "free")
            .unwrap()["endpoints"][0]
            .clone()
    }

    async fn metrics(&self) -> String {
        reqwest::Client::new()
            .get(format!("{}/metrics", self.url))
            .bearer_auth(&self.credential)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn a_429_that_states_a_retry_time_cools_the_endpoint_at_once_for_that_long() {
    let rig = rig(429, "Rate limit reached. Please try again in 20s.", "").await;
    // The first failure falls through to the paid target, and the free endpoint is now cooling even
    // though the breaker needs three failures on its own.
    assert_eq!(rig.ask().await, (200, "paid".to_string()));
    assert_eq!(rig.free.calls(), 1);
    let health = rig.free_health().await;
    assert_eq!(health["state"], "cooling_down");
    assert_eq!(health["cooldown_remaining_ms"], 20_000);
    assert_eq!(health["cooldown_source"], "retry_after");

    // While it cools, the free upstream sees nothing.
    for _ in 0..3 {
        assert_eq!(rig.ask().await.1, "paid");
    }
    assert_eq!(
        rig.free.calls(),
        1,
        "no request reached the cooling endpoint"
    );

    // After exactly the stated time one request probes it, and a success closes the breaker.
    rig.advance_ms(19_999);
    assert_eq!(rig.ask().await.1, "paid");
    assert_eq!(rig.free.calls(), 1, "still cooling one millisecond early");
    rig.advance_ms(1);
    rig.free.answer(200, "");
    assert_eq!(rig.ask().await, (200, "free".to_string()));
    assert_eq!(rig.free.calls(), 2);
    assert_eq!(rig.free_health().await["state"], "healthy");
}

#[tokio::test]
async fn the_stated_time_is_capped_by_the_configured_maximum() {
    let rig = rig(429, "Please try again in 600s", "max_retry_after_secs = 30").await;
    rig.ask().await;
    assert_eq!(rig.free_health().await["cooldown_remaining_ms"], 30_000);
}

#[tokio::test]
async fn a_tiny_stated_time_is_raised_to_one_second() {
    let rig = rig(429, "try again in 50ms", "").await;
    rig.ask().await;
    assert_eq!(rig.free_health().await["cooldown_remaining_ms"], 1_000);
}

#[tokio::test]
async fn a_503_may_state_a_retry_time_too() {
    let rig = rig(503, "overloaded, retry after 15 seconds", "").await;
    rig.ask().await;
    assert_eq!(rig.free_health().await["cooldown_remaining_ms"], 15_000);
}

#[tokio::test]
async fn failures_that_state_nothing_behave_as_before() {
    // A 429 with no readable time counts toward the threshold of three.
    let rig = rig(429, "slow down", "").await;
    for _ in 0..2 {
        rig.ask().await;
    }
    let health = rig.free_health().await;
    assert_eq!(
        health["state"], "healthy",
        "two failures are below the threshold"
    );
    assert_eq!(rig.free.calls(), 2);
    rig.ask().await;
    let health = rig.free_health().await;
    assert_eq!(health["state"], "cooling_down");
    assert_eq!(
        health["cooldown_remaining_ms"], 10_000,
        "the guessed cooldown"
    );
    assert_eq!(health["cooldown_source"], "failures");
}

#[tokio::test]
async fn a_500_that_mentions_a_time_is_not_a_stated_retry_time() {
    let rig = rig(500, "internal error, try again in 20s", "").await;
    rig.ask().await;
    assert_eq!(rig.free_health().await["state"], "healthy");
}

#[tokio::test]
async fn a_stated_time_is_counted_by_provider_and_recorded_on_the_decision() {
    let rig = rig(429, "Please retry in 28.5s", "").await;
    rig.ask().await;
    let text = rig.metrics().await;
    assert!(
        text.contains(r#"humpyard_retry_after_honored_total{provider="pf",source="body"} 1"#),
        "{text}"
    );
    let mut decisions = Value::Null;
    for _ in 0..100 {
        decisions = reqwest::Client::new()
            .get(format!("{}/v1/decisions?route=duo&limit=1", rig.url))
            .bearer_auth(&rig.key)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if decisions["data"].as_array().is_some_and(|d| !d.is_empty()) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
    let attempts = decisions["data"][0]["attempts"]
        .as_array()
        .unwrap_or_else(|| panic!("no attempts in {decisions}"));
    assert_eq!(attempts[0]["outcome"], "rate_limit");
    assert_eq!(attempts[0]["retry_after_ms"], 28_500);
    assert!(
        attempts[1].get("retry_after_ms").is_none(),
        "the paid attempt stated nothing"
    );
    assert_eq!(rig.paid.calls(), 1);
}

#[tokio::test]
async fn a_retry_time_cannot_deny_service() {
    // Every endpoint states a retry time and the only other target is cooling: health still fails open.
    let rig = rig(429, "try again in 60s", "").await;
    rig.paid.answer(429, "try again in 60s");
    let (status, _) = rig.ask().await;
    assert_eq!(
        status, 429,
        "the last upstream's rate limit reaches the client"
    );
    // Both are now cooling, and the next request is still attempted (fail open) instead of refused.
    let before = rig.free.calls() + rig.paid.calls();
    let (status, _) = rig.ask().await;
    assert_eq!(status, 429);
    assert!(
        rig.free.calls() + rig.paid.calls() > before,
        "the request was tried, not refused"
    );
}

#[test]
fn the_cap_must_be_in_range() {
    let check = |extra: &str| {
        Config::from_toml(
            &format!(
                r#"listen = "127.0.0.1:0"
[health]
{extra}
[providers.p]
base_url = "http://127.0.0.1:1"
api_key_env = "K"
[targets.t]
endpoints = [{{ provider = "p", model = "m" }}]
"#
            ),
            |_| Some("k".into()),
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
    };
    assert!(check("max_retry_after_secs = 60").is_ok());
    assert!(
        check("max_retry_after_secs = 0")
            .unwrap_err()
            .contains("max_retry_after_secs")
    );
    assert!(
        check("max_retry_after_secs = 86401")
            .unwrap_err()
            .contains("max_retry_after_secs")
    );
}
