//! Parity with a tiered LiteLLM-style front door: fast, deep and coding groups, each in a public,
//! private and paid tier, with fallbacks between tiers, rotation pools of paid models that have
//! per-model quotas, static provider headers, and keys with a usage ledger.
//!
//! The names are generic stand-ins; the behavior is what a real deployment of this shape relies
//! on. Upstreams are in-process mocks whose behavior each test flips while it runs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use common::{chat_completion, serve, usage};
use humpyard::clock::Clock;
use humpyard::config::Config;
use humpyard::ledger::Ledger;
use humpyard::server::{self, Options};
use serde_json::{Value, json};

// ---- a mock upstream whose behavior can change while a test runs ----

#[derive(Clone)]
enum Reply {
    Ok,
    Status(u16, Value),
    /// Never answers within the provider timeout.
    Stall,
}

type Behavior = Arc<dyn Fn(&Call) -> Reply + Send + Sync>;

struct Call {
    model: String,
    /// Prompt size in bytes: lets a mock refuse prompts that exceed its "context window".
    prompt_bytes: usize,
}

struct Mock {
    behavior: Mutex<Behavior>,
    /// (model, headers) of every call, in order.
    calls: Mutex<Vec<(String, HeaderMap)>>,
}

impl Mock {
    fn set(&self, behavior: impl Fn(&Call) -> Reply + Send + Sync + 'static) {
        *self.behavior.lock().unwrap() = Arc::new(behavior);
    }
    fn models(&self) -> Vec<String> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|c| c.0.clone())
            .collect()
    }
    fn count(&self, model: &str) -> usize {
        self.models().iter().filter(|m| *m == model).count()
    }
    fn total(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
    fn header_of_last_call(&self, name: &str) -> Option<String> {
        let calls = self.calls.lock().unwrap();
        calls
            .last()?
            .1
            .get(name)
            .map(|v| v.to_str().unwrap().to_string())
    }
}

async fn completions(
    State(mock): State<Arc<Mock>>,
    headers: HeaderMap,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let model = body["model"].as_str().unwrap().to_string();
    let prompt_bytes = body["messages"].to_string().len();
    mock.calls.lock().unwrap().push((model.clone(), headers));
    let behavior = mock.behavior.lock().unwrap().clone();
    match behavior(&Call {
        model: model.clone(),
        prompt_bytes,
    }) {
        Reply::Ok => {
            axum::Json(chat_completion(&model, "ok", Some(usage(100, 20)))).into_response()
        }
        Reply::Status(code, body) => {
            (StatusCode::from_u16(code).unwrap(), axum::Json(body)).into_response()
        }
        Reply::Stall => {
            tokio::time::sleep(Duration::from_secs(10)).await;
            StatusCode::OK.into_response()
        }
    }
}

async fn mock() -> (Arc<Mock>, String) {
    let mock = Arc::new(Mock {
        behavior: Mutex::new(Arc::new(|_| Reply::Ok)),
        calls: Mutex::new(vec![]),
    });
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(mock.clone());
    (mock, format!("http://{}", serve(app).await))
}

fn status(code: u16) -> Reply {
    Reply::Status(
        code,
        json!({"error": {"message": format!("status {code}")}}),
    )
}

fn overflow() -> Reply {
    Reply::Status(
        400,
        json!({"error": {"code": "context_length_exceeded", "message": "too long"}}),
    )
}

struct TestClock(AtomicI64);
impl Clock for TestClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

// ---- the rig: three providers and the tier structure ----

struct Rig {
    url: String,
    key: String,
    db: std::path::PathBuf,
    clock: Arc<TestClock>,
    /// Stand-in for the public pool (an aggregator that picks a model itself).
    public: Arc<Mock>,
    /// Stand-in for the private tier (a local model server).
    private: Arc<Mock>,
    /// Stand-in for the paid provider.
    paid: Arc<Mock>,
    _dir: tempfile::TempDir,
}

/// Paid pools, in rotation order. Each group has its own pool, like per-model usage quotas.
const FAST_PAID: [&str; 3] = ["paid-fast-1", "paid-fast-2", "paid-fast-3"];
const DEEP_PAID: [&str; 2] = ["paid-deep-1", "paid-deep-2"];
const CODING_PAID: [&str; 2] = ["paid-code-1", "paid-deep-2"];

fn pool(provider: &str, models: &[&str]) -> String {
    let endpoints: Vec<String> = models
        .iter()
        .map(|m| format!(r#"{{ provider = "{provider}", model = "{m}" }}"#))
        .collect();
    format!("endpoints = [{}]", endpoints.join(", "))
}

/// The gateway config for the three mock providers; `urls` are (public, private, paid).
fn config_toml(db: &std::path::Path, hash: &str, urls: (&str, &str, &str)) -> String {
    let (public_url, private_url, paid_url) = urls;
    format!(
        r#"listen = "127.0.0.1:0"
[ledger]
path = "{db}"
[keys.client]
sha256 = "{hash}"
[health]
failure_threshold = 2
cooldown_secs = 30
max_cooldown_secs = 120

[providers.public]
base_url = "{public_url}"
api_key_env = "K"
max_retries = 0
timeout_secs = 1
[providers.private]
base_url = "{private_url}"
api_key_env = "K"
max_retries = 0
timeout_secs = 1
[providers.paid]
base_url = "{paid_url}"
api_key_env = "K"
max_retries = 0
timeout_secs = 1
headers = {{ "x-session" = "static-session" }}

[targets.pub-fast]
endpoints = [{{ provider = "public", model = "auto-fast" }}]
[targets.pub-deep]
endpoints = [{{ provider = "public", model = "auto-general" }}]
[targets.priv-fast]
endpoints = [{{ provider = "private", model = "local-model" }}]
[targets.priv-deep]
endpoints = [{{ provider = "private", model = "local-model" }}]
[targets.priv-coding]
endpoints = [{{ provider = "private", model = "local-model" }}]
[targets.paid-fast]
{fast_paid}
[targets.spread-1]
endpoints = [{{ provider = "paid", model = "paid-fast-1" }}]
[targets.spread-2]
endpoints = [{{ provider = "paid", model = "paid-fast-2" }}]
[targets.spread-3]
endpoints = [{{ provider = "paid", model = "paid-fast-3" }}]
[targets.paid-deep]
{deep_paid}
[targets.paid-coding]
{coding_paid}

# Tiers fall through in order: a private tier backs onto public, public backs onto paid.
[routes.fast-paid]
type = "passthrough"
targets = ["paid-fast"]
# An alternative to an ordered pool: spread load evenly, other targets are fallbacks.
[routes.fast-paid-spread]
type = "random"
targets = ["spread-1", "spread-2", "spread-3"]
seed = 7
[routes.deep-paid]
type = "passthrough"
targets = ["paid-deep"]
[routes.coding-paid]
type = "passthrough"
targets = ["paid-coding"]
[routes.fast-public]
type = "passthrough"
targets = ["pub-fast", "paid-fast"]
# Strict: paid is a last resort for oversized prompts only, so a public blip never spends credit.
[routes.deep-public]
type = "passthrough"
targets = ["pub-deep", "paid-deep"]
fallback_on = ["overflow"]
[routes.fast-private]
type = "passthrough"
targets = ["priv-fast", "pub-fast", "paid-fast"]
[routes.deep-private]
type = "passthrough"
targets = ["priv-deep", "pub-deep", "paid-deep"]
[routes.coding-private]
type = "passthrough"
targets = ["priv-coding", "paid-coding"]
"#,
        db = db.display(),
        fast_paid = pool("paid", &FAST_PAID),
        deep_paid = pool("paid", &DEEP_PAID),
        coding_paid = pool("paid", &CODING_PAID),
    )
}

async fn rig() -> Rig {
    let (public, public_url) = mock().await;
    let (private, private_url) = mock().await;
    let (paid, paid_url) = mock().await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ledger.db");
    let (key, hash) = humpyard::auth::generate_key().unwrap();
    let toml = config_toml(&db, &hash, (&public_url, &private_url, &paid_url));
    let config = Config::from_toml(&toml, |_| Some("secret".into())).unwrap();
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
        key,
        db,
        clock,
        public,
        private,
        paid,
        _dir: dir,
    }
}

impl Rig {
    async fn chat(&self, model: &str, prompt: &str) -> reqwest::Response {
        reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .bearer_auth(&self.key)
            .json(&json!({"model": model, "messages": [{"role": "user", "content": prompt}]}))
            .send()
            .await
            .unwrap()
    }

    /// Sends a request and returns (status, serving target).
    async fn ask(&self, model: &str, prompt: &str) -> (u16, String) {
        let resp = self.chat(model, prompt).await;
        let status = resp.status().as_u16();
        let target = resp
            .headers()
            .get("x-humpyard-target")
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();
        (status, target)
    }

    fn advance(&self, secs: i64) {
        self.clock.0.fetch_add(secs * 1000, Ordering::SeqCst);
    }
}

// ---- tier selection ----

#[tokio::test]
async fn every_tier_serves_from_its_own_upstream() {
    let r = rig().await;
    assert_eq!(r.ask("fast-public", "hi").await, (200, "pub-fast".into()));
    assert_eq!(r.ask("deep-public", "hi").await, (200, "pub-deep".into()));
    assert_eq!(r.ask("fast-private", "hi").await, (200, "priv-fast".into()));
    assert_eq!(
        r.ask("coding-private", "hi").await,
        (200, "priv-coding".into())
    );
    // Bare targets are routable too, which is how the paid tier is addressed.
    assert_eq!(r.ask("fast-paid", "hi").await, (200, "paid-fast".into()));
    assert_eq!(r.ask("deep-paid", "hi").await, (200, "paid-deep".into()));
    assert_eq!(
        r.ask("coding-paid", "hi").await,
        (200, "paid-coding".into())
    );
    assert_eq!(
        (r.public.total(), r.private.total(), r.paid.total()),
        (2, 2, 3)
    );
    assert_eq!(r.public.models(), ["auto-fast", "auto-general"]);
}

// ---- fallbacks between tiers ----

#[tokio::test]
async fn a_down_private_tier_falls_back_to_public_then_paid() {
    let r = rig().await;
    r.private.set(|_| status(503));
    assert_eq!(r.ask("fast-private", "hi").await, (200, "pub-fast".into()));
    r.public.set(|_| status(503));
    assert_eq!(r.ask("fast-private", "hi").await, (200, "paid-fast".into()));
    assert_eq!(r.ask("deep-private", "hi").await, (200, "paid-deep".into()));
}

#[tokio::test]
async fn a_hung_private_model_falls_back_instead_of_timing_out() {
    let r = rig().await;
    r.private.set(|_| Reply::Stall);
    assert_eq!(r.ask("fast-private", "hi").await, (200, "pub-fast".into()));
}

#[tokio::test]
async fn private_coding_falls_back_to_paid_coding() {
    let r = rig().await;
    r.private.set(|_| status(500));
    assert_eq!(
        r.ask("coding-private", "hi").await,
        (200, "paid-coding".into())
    );
    assert_eq!(r.paid.models(), ["paid-code-1"]);
}

#[tokio::test]
async fn a_prompt_too_large_for_the_public_tier_goes_to_paid() {
    let r = rig().await;
    // The public pool only accepts small prompts (its "context window"); paid takes anything.
    r.public.set(|c| {
        if c.prompt_bytes > 2_000 {
            overflow()
        } else {
            Reply::Ok
        }
    });
    assert_eq!(
        r.ask("fast-public", "short").await,
        (200, "pub-fast".into())
    );
    let big = "x".repeat(5_000);
    assert_eq!(r.ask("fast-public", &big).await, (200, "paid-fast".into()));
    assert_eq!(r.ask("deep-public", &big).await, (200, "paid-deep".into()));
}

#[tokio::test]
async fn a_strict_route_spends_paid_credit_only_on_overflow() {
    let r = rig().await;
    // The public tier is flaky: a lenient route (fast-public) would fall through to paid...
    r.public.set(|_| status(503));
    assert_eq!(r.ask("fast-public", "hi").await, (200, "paid-fast".into()));
    // ...a strict one (deep-public) reports the failure and keeps paid untouched.
    let paid_before = r.paid.total();
    assert_eq!(r.ask("deep-public", "hi").await.0, 502);
    assert_eq!(
        r.paid.total(),
        paid_before,
        "no paid call for a public blip"
    );
    r.public.set(|_| status(429));
    assert_eq!(r.ask("deep-public", "hi").await.0, 429);
    // Oversized prompts are the one thing it does hand to paid.
    r.public.set(|_| overflow());
    assert_eq!(r.ask("deep-public", "hi").await, (200, "paid-deep".into()));
}

#[tokio::test]
async fn client_errors_do_not_trigger_a_fallback() {
    let r = rig().await;
    r.public
        .set(|_| Reply::Status(400, json!({"error": {"message": "bad request"}})));
    assert_eq!(r.ask("fast-public", "hi").await.0, 400);
    assert_eq!(
        r.paid.total(),
        0,
        "a malformed request must not spend paid credits"
    );
}

#[tokio::test]
async fn everything_down_returns_the_last_error() {
    let r = rig().await;
    r.public.set(|_| status(503));
    r.private.set(|_| status(503));
    r.paid.set(|_| status(503));
    let (code, target) = r.ask("fast-private", "hi").await;
    assert_eq!(code, 502);
    assert_eq!(target, "");
}

// ---- paid rotation pools with per-model quotas ----

#[tokio::test]
async fn a_paid_model_over_its_quota_is_rotated_out_and_comes_back() {
    let r = rig().await;
    // The first model's quota is exhausted (429); the others are fine.
    r.paid.set(|c| {
        if c.model == "paid-fast-1" {
            status(429)
        } else {
            Reply::Ok
        }
    });

    // Until the breaker opens (2 consecutive failures) each request tries model 1 first.
    for _ in 0..2 {
        assert_eq!(r.ask("fast-paid", "hi").await, (200, "paid-fast".into()));
    }
    assert_eq!(r.paid.count("paid-fast-1"), 2);
    assert_eq!(r.paid.count("paid-fast-2"), 2);

    // Now it is skipped outright: the exhausted quota is no longer hammered.
    for _ in 0..5 {
        assert_eq!(r.ask("fast-paid", "hi").await.0, 200);
    }
    assert_eq!(r.paid.count("paid-fast-1"), 2, "skipped while cooling down");
    assert_eq!(r.paid.count("paid-fast-2"), 7);

    // The quota resets; after the cooldown one probe finds it healthy and it rejoins.
    r.paid.set(|_| Reply::Ok);
    r.advance(31);
    r.ask("fast-paid", "hi").await;
    r.ask("fast-paid", "hi").await;
    assert_eq!(r.paid.count("paid-fast-1"), 4, "back in rotation first");
}

#[tokio::test]
async fn when_every_model_in_a_pool_is_over_quota_the_client_sees_a_rate_limit() {
    let r = rig().await;
    r.paid.set(|_| status(429));
    let (code, _) = r.ask("deep-paid", "hi").await;
    assert_eq!(code, 429);
    // Both models were tried before giving up.
    assert_eq!(r.paid.models(), ["paid-deep-1", "paid-deep-2"]);
}

#[tokio::test]
async fn pools_that_share_a_model_share_its_health_separately_per_target() {
    // A model listed in two groups is tracked per target, so one group's trouble with it does not
    // silently remove it from the other group (each pool is judged on its own traffic).
    let r = rig().await;
    r.paid.set(|c| {
        if c.model == "paid-deep-2" {
            status(500)
        } else {
            Reply::Ok
        }
    });
    for _ in 0..3 {
        r.ask("coding-paid", "hi").await; // paid-code-1 serves; paid-deep-2 is never needed
    }
    assert_eq!(r.paid.count("paid-deep-2"), 0);
    r.paid.set(|c| {
        if c.model == "paid-code-1" {
            status(500)
        } else {
            Reply::Ok
        }
    });
    assert_eq!(r.ask("coding-paid", "hi").await.0, 200);
    assert_eq!(r.paid.models().last().unwrap(), "paid-deep-2");
}

// ---- headers, keys and usage ----

#[tokio::test]
async fn a_providers_static_header_goes_only_to_that_provider() {
    let r = rig().await;
    r.ask("fast-paid", "hi").await;
    r.ask("fast-public", "hi").await;
    assert_eq!(
        r.paid.header_of_last_call("x-session").as_deref(),
        Some("static-session")
    );
    assert_eq!(r.public.header_of_last_call("x-session"), None);
    // And each provider gets its own credential, never the client's gateway key.
    assert_eq!(
        r.paid.header_of_last_call("authorization").as_deref(),
        Some("Bearer secret")
    );
    assert!(
        !r.paid
            .header_of_last_call("authorization")
            .unwrap()
            .contains("sk-humpyard")
    );
}

#[tokio::test]
async fn requests_need_a_key_and_usage_is_recorded_per_route_target_and_model() {
    let r = rig().await;
    let anonymous = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", r.url))
        .json(&json!({"model": "fast-public", "messages": [{"role": "user", "content": "hi"}]}))
        .send()
        .await
        .unwrap();
    assert_eq!(anonymous.status(), 401);

    r.public.set(|_| status(503)); // fast-public falls to paid
    r.ask("fast-public", "hi").await;
    r.ask("deep-private", "hi").await;
    tokio::time::sleep(Duration::from_millis(400)).await;

    let rows = Ledger::open(&r.db)
        .await
        .unwrap()
        .entries_since(0)
        .await
        .unwrap();
    let by: HashMap<(String, String), String> = rows
        .iter()
        .map(|e| ((e.route.clone(), e.target.clone()), e.model.clone()))
        .collect();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert_eq!(
        by[&("fast-public".into(), "paid-fast".into())],
        "paid-fast-1"
    );
    assert_eq!(
        by[&("deep-private".into(), "priv-deep".into())],
        "local-model"
    );
    assert!(
        rows.iter()
            .all(|e| e.key_id.as_deref() == Some("client") && e.output_tokens == 20)
    );
}

#[tokio::test]
async fn a_random_route_spreads_load_across_the_pool_and_routes_around_a_limited_model() {
    let r = rig().await;
    for _ in 0..30 {
        assert_eq!(r.ask("fast-paid-spread", "hi").await.0, 200);
    }
    let counts: Vec<usize> = FAST_PAID.iter().map(|m| r.paid.count(m)).collect();
    assert!(
        counts.iter().all(|&c| c >= 3),
        "every model gets traffic: {counts:?}"
    );

    // One model hits its quota: its requests are served by the others, with no client errors.
    r.paid.set(|c| {
        if c.model == "paid-fast-2" {
            status(429)
        } else {
            Reply::Ok
        }
    });
    for _ in 0..30 {
        assert_eq!(r.ask("fast-paid-spread", "hi").await.0, 200);
    }
}
