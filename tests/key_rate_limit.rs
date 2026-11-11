//! The per-key request-rate limit (`max_requests_per_minute`): the N+1th request in a minute is refused with
//! 429 before any upstream call, after authentication and the route check and before the budget.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::post;
use common::{chat_completion, serve, usage};
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

struct Upstream {
    calls: AtomicUsize,
}

async fn completions(State(up): State<Arc<Upstream>>) -> axum::response::Response {
    up.calls.fetch_add(1, Ordering::SeqCst);
    axum::Json(chat_completion("m", "ok", Some(usage(10, 5)))).into_response()
}

struct Rig {
    url: String,
    clock: Arc<TestClock>,
    upstream: Arc<Upstream>,
    metrics_credential: String,
    /// Three requests a minute, a roomy budget.
    limited: String,
    /// The same limit, a second key: its allowance is its own.
    neighbour: String,
    /// One request a minute and a token budget a single call uses up.
    tight: String,
    /// No limit at all.
    free: String,
    _dir: tempfile::TempDir,
}

async fn rig() -> Rig {
    let upstream = Arc::new(Upstream {
        calls: AtomicUsize::new(0),
    });
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(upstream.clone());
    let base = format!("http://{}", serve(app).await);
    let dir = tempfile::tempdir().unwrap();
    let (limited, limited_hash) = humpyard::auth::generate_key().unwrap();
    let (neighbour, neighbour_hash) = humpyard::auth::generate_key().unwrap();
    let (tight, tight_hash) = humpyard::auth::generate_key().unwrap();
    let (free, free_hash) = humpyard::auth::generate_key().unwrap();
    let (metrics_credential, metrics_hash) = humpyard::auth::generate_key().unwrap();
    let toml = format!(
        r#"listen = "127.0.0.1:0"
# The test clock sits in 1970; retention would prune each decision once it is written.
decision_retention_days = 0
[ledger]
path = "{db}"
[metrics]
enabled = true
bearer_sha256 = "{metrics_hash}"
[keys.limited]
sha256 = "{limited_hash}"
max_requests_per_minute = 3
daily_tokens = 100000
[keys.neighbour]
sha256 = "{neighbour_hash}"
max_requests_per_minute = 3
[keys.tight]
sha256 = "{tight_hash}"
max_requests_per_minute = 1
daily_tokens = 15
[keys.free]
sha256 = "{free_hash}"
[providers.p]
base_url = "{base}"
api_key_env = "K"
max_retries = 0
[targets.t]
endpoints = [{{ provider = "p", model = "m" }}]
[routes.fast]
type = "passthrough"
targets = ["t"]
"#,
        db = dir.path().join("ledger.db").display(),
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
    Rig {
        url: format!("http://{}", serve(router).await),
        clock,
        upstream,
        metrics_credential,
        limited,
        neighbour,
        tight,
        free,
        _dir: dir,
    }
}

impl Rig {
    async fn chat(&self, key: &str) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .bearer_auth(key)
            .json(&json!({"model": "fast", "messages": [{"role": "user", "content": "hi"}]}))
            .send()
            .await
            .unwrap()
    }

    async fn status(&self, key: &str) -> u16 {
        self.chat(key).await.status().as_u16()
    }

    async fn get(&self, path: &str, key: &str) -> reqwest::Response {
        reqwest::Client::new()
            .get(format!("{}{path}", self.url))
            .bearer_auth(key)
            .send()
            .await
            .unwrap()
    }

    fn advance(&self, ms: i64) {
        self.clock.0.fetch_add(ms, Ordering::SeqCst);
    }

    fn calls(&self) -> usize {
        self.upstream.calls.load(Ordering::SeqCst)
    }

    async fn decisions_with(&self, key: &str, outcome: &str) -> Vec<Value> {
        for _ in 0..100 {
            let body: Value = self
                .get("/v1/decisions?limit=100", key)
                .await
                .json()
                .await
                .unwrap();
            let found: Vec<Value> = body["data"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|d| d["outcome"] == outcome)
                .cloned()
                .collect();
            if !found.is_empty() {
                return found;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        panic!("no `{outcome}` decision");
    }

    async fn metrics(&self) -> String {
        self.get("/metrics", &self.metrics_credential)
            .await
            .text()
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn the_request_after_the_limit_is_refused_with_429_and_makes_no_upstream_call() {
    let rig = rig().await;
    for _ in 0..3 {
        assert_eq!(rig.status(&rig.limited).await, 200);
    }
    assert_eq!(rig.calls(), 3);
    let refused = rig.chat(&rig.limited).await;
    assert_eq!(refused.status(), 429);
    assert!(refused.headers().contains_key("x-humpyard-request-id"));
    let body: Value = refused.json().await.unwrap();
    assert_eq!(body["error"]["code"], "rate_limit_exceeded");
    assert_eq!(body["error"]["type"], "rate_limit_error");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("at most 3 requests per minute"),
        "{body}"
    );
    assert_eq!(rig.calls(), 3, "the refused request reached no upstream");
}

#[tokio::test]
async fn retry_after_is_the_time_until_the_oldest_request_leaves_the_window() {
    let rig = rig().await;
    assert_eq!(rig.status(&rig.limited).await, 200); // t = 0
    rig.advance(10_000);
    assert_eq!(rig.status(&rig.limited).await, 200); // t = 10 s
    assert_eq!(rig.status(&rig.limited).await, 200); // t = 10 s
    rig.advance(20_400); // t = 30.4 s: the oldest leaves at 60 s, 29.6 s away
    let refused = rig.chat(&rig.limited).await;
    assert_eq!(refused.status(), 429);
    assert_eq!(refused.headers()["retry-after"], "30");
}

#[tokio::test]
async fn a_slid_window_admits_again() {
    let rig = rig().await;
    for _ in 0..3 {
        assert_eq!(rig.status(&rig.limited).await, 200);
    }
    assert_eq!(rig.status(&rig.limited).await, 429);
    rig.advance(59_999);
    assert_eq!(rig.status(&rig.limited).await, 429);
    rig.advance(1);
    assert_eq!(rig.status(&rig.limited).await, 200);
}

#[tokio::test]
async fn the_limit_is_checked_before_the_budget_and_a_refusal_spends_nothing() {
    let rig = rig().await;
    // One call uses the key's whole token budget (15 tokens).
    assert_eq!(rig.status(&rig.tight).await, 200);
    let spent = rig
        .get("/v1/key/info", &rig.tight)
        .await
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(spent["spend"]["daily_tokens"], 15);
    // The budget is now gone, but the rate is checked first: 429, not 402.
    assert_eq!(rig.status(&rig.tight).await, 429);
    assert_eq!(rig.status(&rig.tight).await, 429);
    let after = rig
        .get("/v1/key/info", &rig.tight)
        .await
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(
        after["spend"], spent["spend"],
        "a rate-limited request spends nothing"
    );
    // Once the window has slid the budget check is reached.
    rig.advance(60_000);
    assert_eq!(rig.status(&rig.tight).await, 402);
}

#[tokio::test]
async fn the_refusal_is_a_decision_and_a_metric() {
    let rig = rig().await;
    for _ in 0..4 {
        rig.status(&rig.limited).await;
    }
    let decisions = rig.decisions_with(&rig.limited, "rate_limited").await;
    assert_eq!(decisions.len(), 1);
    assert_eq!(decisions[0]["status"], 429);
    assert_eq!(decisions[0]["key"], "limited");
    assert_eq!(
        decisions[0]["route"], "fast",
        "the route was authorised before the limit"
    );
    assert_eq!(decisions[0]["attempts"], json!([]));
    let metrics = rig.metrics().await;
    assert!(
        metrics.lines().any(
            |l| l.starts_with("humpyard_refusals_total{reason=\"rate_limited\"")
                && l.ends_with(" 1")
        ),
        "{metrics}"
    );
}

#[tokio::test]
async fn another_key_and_a_key_without_a_limit_are_unaffected() {
    let rig = rig().await;
    for _ in 0..4 {
        rig.status(&rig.limited).await;
    }
    assert_eq!(rig.status(&rig.limited).await, 429);
    for _ in 0..3 {
        assert_eq!(rig.status(&rig.neighbour).await, 200);
    }
    for _ in 0..20 {
        assert_eq!(rig.status(&rig.free).await, 200);
    }
}

#[tokio::test]
async fn a_failed_authentication_changes_no_counter() {
    let rig = rig().await;
    for _ in 0..10 {
        assert_eq!(rig.status("sk-humpyard-not-a-key").await, 401);
    }
    for _ in 0..3 {
        assert_eq!(rig.status(&rig.limited).await, 200);
    }
}

#[tokio::test]
async fn reading_statistics_is_not_limited() {
    let rig = rig().await;
    for _ in 0..4 {
        rig.status(&rig.limited).await;
    }
    assert_eq!(rig.status(&rig.limited).await, 429);
    for path in ["/v1/stats", "/v1/key/info", "/v1/decisions", "/v1/models"] {
        assert_eq!(
            rig.get(path, &rig.limited).await.status(),
            200,
            "{path} at the request limit"
        );
    }
}

#[tokio::test]
async fn a_refusal_on_the_anthropic_endpoint_has_that_error_shape_and_retry_after() {
    let rig = rig().await;
    let send = || async {
        reqwest::Client::new()
            .post(format!("{}/v1/messages", rig.url))
            .bearer_auth(&rig.tight)
            .header("anthropic-version", "2023-06-01")
            .json(&json!({"model": "fast", "max_tokens": 16, "messages": [{"role": "user", "content": "hi"}]}))
            .send()
            .await
            .unwrap()
    };
    send().await;
    let refused = send().await;
    assert_eq!(refused.status(), 429);
    assert_eq!(refused.headers()["retry-after"], "60");
    let body: Value = refused.json().await.unwrap();
    assert_eq!(body["type"], "error");
    assert_eq!(body["error"]["type"], "rate_limit_error");
}
