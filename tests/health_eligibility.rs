//! Health-driven eligibility (ADR 0018): a target whose endpoints are all cooling down is skipped when
//! another target remains, health never denies service, a cooled target is re-admitted by its probe,
//! and strict routes and the off switch behave.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU16, AtomicUsize, Ordering};
use std::time::Duration;

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

struct Upstream {
    status: AtomicU16,
    calls: AtomicUsize,
}

impl Upstream {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
    fn set(&self, status: u16) {
        self.status.store(status, Ordering::SeqCst);
    }
}

async fn completions(State(up): State<Arc<Upstream>>) -> Response {
    up.calls.fetch_add(1, Ordering::SeqCst);
    match up.status.load(Ordering::SeqCst) {
        200 => axum::Json(chat_completion("m", "ok", Some(usage(10, 5)))).into_response(),
        code => (
            StatusCode::from_u16(code).unwrap(),
            axum::Json(json!({"error": {"message": "down"}})),
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

struct Rig {
    url: String,
    key: String,
    credential: String,
    clock: Arc<TestClock>,
    first: Arc<Upstream>,
    spare: Arc<Upstream>,
    public: Arc<Upstream>,
    _dir: tempfile::TempDir,
}

/// Targets `a` and `a2` (class private) and `b` (class public), each on its own mock upstream.
/// Routes: `duo` (a, b), `tiered` (stage router: efficient a, capable b), `strict` (a, a2, b, only
/// private allowed), `lone` (a only). `a` starts failing with `a_status`; one failure trips its
/// breaker (threshold 1, cooldown 60 s on a controllable clock).
async fn rig(a_status: u16, health_extra: &str) -> Rig {
    let (first, first_url) = upstream(a_status).await;
    let (spare, spare_url) = upstream(200).await;
    let (public, public_url) = upstream(200).await;
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
failure_threshold = 1
cooldown_secs = 60
max_cooldown_secs = 60
{health_extra}
[keys.alice]
sha256 = "{key_hash}"
[providers.pa]
base_url = "{first_url}"
api_key_env = "K"
max_retries = 0
[providers.pa2]
base_url = "{spare_url}"
api_key_env = "K"
max_retries = 0
[providers.pb]
base_url = "{public_url}"
api_key_env = "K"
max_retries = 0
[targets.a]
class = "private"
endpoints = [{{ provider = "pa", model = "m" }}]
[targets.a2]
class = "private"
endpoints = [{{ provider = "pa2", model = "m" }}]
[targets.b]
class = "public"
endpoints = [{{ provider = "pb", model = "m" }}]
[routes.duo]
type = "passthrough"
targets = ["a", "b"]
[routes.tiered]
type = "stage_router"
efficient = ["a"]
capable = ["b"]
[routes.strict]
type = "passthrough"
targets = ["a", "a2", "b"]
allowed_classes = ["private"]
[routes.lone]
type = "passthrough"
targets = ["a"]
"#,
        db = dir.path().join("ledger.db").display(),
    );
    let clock = Arc::new(TestClock(AtomicI64::new(1_000_000)));
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
        first,
        spare,
        public,
        _dir: dir,
    }
}

impl Rig {
    /// Status and the serving target's name (empty when the request failed).
    async fn ask(&self, route: &str) -> (u16, String) {
        let resp = reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .bearer_auth(&self.key)
            .json(&chat_request(route, false))
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

    fn advance(&self, secs: i64) {
        self.clock.0.fetch_add(secs * 1000, Ordering::SeqCst);
    }

    async fn explain(&self, route: &str) -> Value {
        reqwest::Client::new()
            .post(format!("{}/v1/route/explain", self.url))
            .bearer_auth(&self.key)
            .json(&json!({"model": route}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

    /// The newest decision for `route`, waiting for the ledger's asynchronous writer.
    async fn decision(&self, route: &str) -> Value {
        for _ in 0..100 {
            let body: Value = reqwest::Client::new()
                .get(format!("{}/v1/decisions?route={route}&limit=1", self.url))
                .bearer_auth(&self.key)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            if let Some(first) = body["data"].as_array().and_then(|d| d.first()) {
                return first.clone();
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        panic!("no decision for {route}");
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
async fn a_cold_target_gets_no_attempt_while_an_alternative_is_warm() {
    let rig = rig(503, "").await;
    // The first request finds `a` down, fails over to `b`, and trips a's breaker.
    assert_eq!(rig.ask("duo").await, (200, "b".into()));
    assert_eq!(rig.first.calls(), 1);

    // `a` is cold now: the request goes straight to `b` and `a` is not called.
    assert_eq!(rig.ask("duo").await, (200, "b".into()));
    assert_eq!(
        rig.first.calls(),
        1,
        "no attempt is spent on the cold target"
    );
    assert_eq!(rig.public.calls(), 2);

    let decision = rig.decision("duo").await;
    assert_eq!(decision["cold_targets"], json!(["a"]));
    assert_eq!(decision["eligible_targets"], json!(["b"]));
    let attempts = decision["attempts"].as_array().unwrap();
    assert!(attempts.iter().all(|t| t["target"] == "b"), "{attempts:?}");

    let text = rig.metrics().await;
    assert!(
        text.contains(r#"humpyard_health_skips_total{route="duo",target="a"} 1"#),
        "{text}"
    );
}

#[tokio::test]
async fn health_never_denies_service_when_everything_is_cold() {
    let rig = rig(503, "").await;
    rig.public.set(503);
    let (status, _) = rig.ask("duo").await;
    assert!(
        status >= 500,
        "both are down, so the request fails: {status}"
    );
    assert_eq!((rig.first.calls(), rig.public.calls()), (1, 1));

    // Both are cold now. They are still tried, in order, because nothing is left to skip to.
    let (status, _) = rig.ask("duo").await;
    assert!(status >= 500);
    assert_eq!((rig.first.calls(), rig.public.calls()), (2, 2));
    let decision = rig.decision("duo").await;
    assert_eq!(decision["cold_targets"], json!([]));

    let explain = rig.explain("duo").await;
    assert_eq!(explain["health_fail_open"], json!(true));
    assert_eq!(explain["health_skipped_targets"], json!([]));
    assert!(!rig.metrics().await.contains("humpyard_health_skips_total{"));

    // And a recovered one serves at once.
    rig.first.set(200);
    rig.advance(61);
    assert_eq!(rig.ask("duo").await, (200, "a".into()));
}

#[tokio::test]
async fn a_cooled_target_is_probed_and_recovers() {
    let rig = rig(503, "").await;
    assert_eq!(rig.ask("duo").await, (200, "b".into()));
    assert_eq!(rig.ask("duo").await, (200, "b".into()));
    assert_eq!(rig.first.calls(), 1, "skipped while cooling down");

    rig.first.set(200);
    rig.advance(61); // the cooldown ended: the next request is the probe
    assert_eq!(rig.ask("duo").await, (200, "a".into()));
    assert_eq!(rig.first.calls(), 2);
    assert_eq!(rig.ask("duo").await, (200, "a".into()), "back in service");
    assert_eq!(
        rig.explain("duo").await["health_skipped_targets"],
        json!([])
    );
}

#[tokio::test]
async fn a_failed_probe_keeps_the_target_out_for_the_next_cooldown() {
    let rig = rig(503, "").await;
    rig.ask("duo").await;
    rig.advance(61);
    assert_eq!(
        rig.ask("duo").await,
        (200, "b".into()),
        "the probe fails, b serves"
    );
    assert_eq!(rig.first.calls(), 2);
    assert_eq!(rig.ask("duo").await, (200, "b".into()));
    assert_eq!(rig.first.calls(), 2, "cooling down again");
}

#[tokio::test]
async fn a_cold_tier_is_served_by_the_other_tier() {
    let rig = rig(503, "").await;
    // The stage router tries `efficient` (a) first, which fails; b takes over and a goes cold.
    assert_eq!(rig.ask("tiered").await, (200, "b".into()));
    assert_eq!(rig.first.calls(), 1);
    assert_eq!(rig.ask("tiered").await, (200, "b".into()));
    assert_eq!(
        rig.first.calls(),
        1,
        "the cold efficient tier gets no attempt"
    );
}

#[tokio::test]
async fn a_strict_route_skips_only_within_its_allowed_targets() {
    let rig = rig(503, "").await;
    assert_eq!(rig.ask("strict").await, (200, "a2".into()));
    assert_eq!(rig.ask("strict").await, (200, "a2".into()));
    assert_eq!(rig.first.calls(), 1, "a is cold and skipped");
    assert_eq!(
        rig.public.calls(),
        0,
        "the excluded public target is never called"
    );
    assert_eq!(
        rig.spare.calls(),
        2,
        "the other private target served both requests"
    );
    let decision = rig.decision("strict").await;
    assert_eq!(decision["cold_targets"], json!(["a"]));
    assert_eq!(decision["excluded_targets"], json!(["b"]));
}

#[tokio::test]
async fn a_single_cold_target_is_still_tried() {
    let rig = rig(503, "").await;
    let (status, _) = rig.ask("lone").await;
    assert!(status >= 500);
    let (status, _) = rig.ask("lone").await;
    assert!(status >= 500);
    assert_eq!(rig.first.calls(), 2, "the only target is tried every time");
}

#[tokio::test]
async fn the_switch_restores_the_old_behavior() {
    let rig = rig(503, "skip_cold_targets = false").await;
    assert_eq!(rig.ask("duo").await, (200, "b".into()));
    assert_eq!(rig.ask("duo").await, (200, "b".into()));
    assert_eq!(rig.first.calls(), 2, "the cold target is still tried first");
    assert_eq!(rig.decision("duo").await["cold_targets"], json!([]));
}

#[tokio::test]
async fn explain_marks_each_target_cold_or_warm() {
    let rig = rig(503, "").await;
    rig.ask("duo").await;
    let explain = rig.explain("duo").await;
    assert_eq!(explain["health_skipped_targets"], json!(["a"]));
    assert_eq!(explain["health_fail_open"], json!(false));
    let planned: Vec<&str> = explain["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["target"].as_str().unwrap())
        .collect();
    assert_eq!(planned, vec!["b"]);
    assert_eq!(explain["targets"][0]["cold"], json!(false));
}

#[tokio::test]
async fn an_older_ledger_is_upgraded_and_keeps_its_decisions() {
    use sqlx::Connection;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    humpyard::ledger::Ledger::open(&path)
        .await
        .unwrap()
        .shutdown()
        .await;
    // A version-4 file: no cold_targets column, one legacy decision dated now (the ledger prunes
    // old decisions shortly after opening). Built offline so no live pool holds a stale statement.
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&path);
    let mut conn = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    for statement in [
        "ALTER TABLE decisions DROP COLUMN cold_targets",
        "INSERT INTO decisions (request_id, ts_ms, format, requested_model, skipped_rules, \
         eligible_targets, stream, tools, images, attempts, status, outcome, latency_ms) \
         VALUES ('before', CAST(strftime('%s', 'now') AS INTEGER) * 1000, 'openai_chat', 'm', \
         '[]', '[]', 0, 0, 0, '[]', 200, 'ok', 5)",
        "PRAGMA user_version = 4",
    ] {
        sqlx::query(statement).execute(&mut conn).await.unwrap();
    }
    conn.close().await.unwrap();

    let ledger = humpyard::ledger::Ledger::open(&path).await.unwrap();
    ledger.record_decision(humpyard::ledger::Decision {
        request_id: "after".into(),
        ts_ms: humpyard::clock::SystemClock.now_ms(),
        format: "openai_chat".into(),
        outcome: "ok".into(),
        status: 200,
        cold_targets: vec!["a".into()],
        ..humpyard::ledger::Decision::default()
    });
    ledger.flush().await;
    let rows = ledger
        .decisions(&humpyard::ledger::DecisionQuery {
            limit: 10,
            ..humpyard::ledger::DecisionQuery::default()
        })
        .await
        .unwrap();
    let by_id = |id: &str| rows.iter().find(|d| d.request_id == id).unwrap();
    assert_eq!(by_id("before").cold_targets, Vec::<String>::new());
    assert_eq!(by_id("after").cold_targets, vec!["a".to_string()]);
}
