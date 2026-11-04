//! Routing statistics: failure mix, fallback rate, percentiles, cost per request, per-key
//! isolation, windows and `explain` annotations, all read-only over the decision log and usage rows.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use common::{chat_completion, chat_request, serve, usage};
use humpyard::config::Config;
use humpyard::ledger::{Attempt, Decision, GroupBy, Ledger, StatsQuery, percentile};
use humpyard::server;
use serde_json::{Value, json};

struct Upstream {
    status: AtomicU16,
}

async fn completions(State(up): State<Arc<Upstream>>) -> axum::response::Response {
    match up.status.load(Ordering::SeqCst) {
        200 => axum::Json(chat_completion("m", "ok", Some(usage(10, 5)))).into_response(),
        code => (
            StatusCode::from_u16(code).unwrap(),
            axum::Json(json!({"error": {"message": "nope"}})),
        )
            .into_response(),
    }
}

async fn upstream(status: u16) -> (Arc<Upstream>, String) {
    let up = Arc::new(Upstream {
        status: AtomicU16::new(status),
    });
    let app = Router::new()
        .route("/chat/completions", post(completions))
        .with_state(up.clone());
    (up, format!("http://{}", serve(app).await))
}

struct Rig {
    url: String,
    db: std::path::PathBuf,
    keys: HashMap<&'static str, String>,
    first: Arc<Upstream>,
    _dir: tempfile::TempDir,
}

/// Targets `a` (priced: input $1, output $2 per million) and `b`; routes `plain` (a) and
/// `fallback` (a then b); keys `alice` and `bob`.
async fn rig() -> Rig {
    let (first, a_url) = upstream(200).await;
    let (_second, b_url) = upstream(200).await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ledger.db");
    let mut keys = HashMap::new();
    let mut blocks = Vec::new();
    for id in ["alice", "bob"] {
        let (key, hash) = humpyard::auth::generate_key().unwrap();
        blocks.push(format!("[keys.{id}]\nsha256 = \"{hash}\""));
        keys.insert(id, key);
    }
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[ledger]
path = "{db}"
# These tests are about statistics, not health: keep every target in the plan so a target that
# failed is still tried (health skipping has its own tests in health_eligibility.rs).
[health]
skip_cold_targets = false
{keys}
[providers.pa]
base_url = "{a_url}"
api_key_env = "K"
max_retries = 0
[providers.pb]
base_url = "{b_url}"
api_key_env = "K"
max_retries = 0
[targets.a]
endpoints = [{{ provider = "pa", model = "ma", price = {{ input = 1.0, output = 2.0 }} }}]
[targets.b]
endpoints = [{{ provider = "pb", model = "mb" }}]
[routes.plain]
type = "passthrough"
targets = ["a"]
[routes.fallback]
type = "passthrough"
targets = ["a", "b"]
"#,
        db = db.display(),
        keys = blocks.join("\n")
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    Rig {
        url: format!(
            "http://{}",
            serve(server::router(config).await.unwrap()).await
        ),
        db,
        keys,
        first,
        _dir: dir,
    }
}

impl Rig {
    async fn ask(&self, key: &str, model: &str) -> u16 {
        reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .bearer_auth(&self.keys[key])
            .json(&chat_request(model, false))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    }

    async fn stats(&self, key: &str, query: &str) -> Value {
        let resp = reqwest::Client::new()
            .get(format!("{}/v1/stats{query}", self.url))
            .bearer_auth(&self.keys[key])
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "{query}");
        resp.json().await.unwrap()
    }
}

fn group<'a>(stats: &'a Value, name: &str) -> &'a Value {
    stats["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["group"] == name)
        .unwrap_or_else(|| panic!("no group {name} in {stats}"))
}

#[tokio::test]
async fn failure_mix_and_fallback_rate_show_what_a_target_has_been_doing() {
    let r = rig().await;
    r.first.status.store(429, Ordering::SeqCst);
    for _ in 0..3 {
        assert_eq!(
            r.ask("alice", "fallback").await,
            200,
            "b serves while a is limited"
        );
    }
    r.first.status.store(200, Ordering::SeqCst);
    for _ in 0..2 {
        assert_eq!(r.ask("alice", "fallback").await, 200);
    }

    let by_target = r.stats("alice", "?group_by=target").await;
    let a = group(&by_target, "a");
    assert_eq!(
        (a["attempts"].as_u64(), a["failures"].as_u64()),
        (Some(5), Some(3))
    );
    assert_eq!(a["outcomes"], json!({"ok": 2, "rate_limit": 3}));
    assert!((a["failure_rate"].as_f64().unwrap() - 0.6).abs() < 1e-9);
    let b = group(&by_target, "b");
    assert_eq!(
        (b["attempts"].as_u64(), b["failures"].as_u64()),
        (Some(3), Some(0))
    );

    let by_route = r.stats("alice", "").await; // route is the default grouping
    let route = group(&by_route, "fallback");
    assert_eq!(route["requests"], 5);
    assert_eq!(route["fallbacks"], 3);
    assert!((route["fallback_rate"].as_f64().unwrap() - 0.6).abs() < 1e-9);

    let by_endpoint = r.stats("alice", "?group_by=endpoint").await;
    assert_eq!(group(&by_endpoint, "pa/ma")["attempts"], 5);
    assert_eq!(group(&by_endpoint, "pb/mb")["attempts"], 3);

    // The target filter narrows a route's numbers to one target's attempts.
    let filtered = r.stats("alice", "?target=a").await;
    assert_eq!(group(&filtered, "fallback")["attempts"], 5);
}

#[tokio::test]
async fn cost_and_tokens_per_answered_request_come_from_the_usage_rows() {
    let r = rig().await;
    for _ in 0..4 {
        assert_eq!(r.ask("alice", "plain").await, 200);
    }
    // 10 input tokens at $1/M and 5 output tokens at $2/M is 20 micro-USD, 15 tokens.
    let stats = r.stats("alice", "").await;
    let plain = group(&stats, "plain");
    assert_eq!(plain["answered"], 4);
    assert!((plain["avg_cost_usd"].as_f64().unwrap() - 0.000_02).abs() < 1e-12);
    assert!((plain["avg_tokens"].as_f64().unwrap() - 15.0).abs() < 1e-9);
    // Unpriced target b costs nothing.
    assert_eq!(r.ask("alice", "fallback").await, 200); // served by a
    r.first.status.store(500, Ordering::SeqCst);
    assert_eq!(r.ask("alice", "fallback").await, 200); // served by b
    let by_target = r.stats("alice", "?group_by=target").await;
    assert_eq!(
        group(&by_target, "b")["avg_cost_usd"].as_f64().unwrap(),
        0.0
    );
}

#[tokio::test]
async fn a_key_sees_only_its_own_statistics() {
    let r = rig().await;
    for _ in 0..3 {
        r.ask("alice", "plain").await;
    }
    r.ask("bob", "plain").await;
    assert_eq!(group(&r.stats("alice", "").await, "plain")["requests"], 3);
    assert_eq!(group(&r.stats("bob", "").await, "plain")["requests"], 1);
}

#[tokio::test]
async fn windows_parameters_and_empty_results_behave() {
    let r = rig().await;
    // Nothing yet: 200 with no groups.
    let empty = r.stats("alice", "?window=1h").await;
    assert_eq!(empty["data"], json!([]));
    assert_eq!(empty["truncated"], false);
    r.ask("alice", "plain").await;
    assert_eq!(
        r.stats("alice", "?window=15m").await["data"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        r.stats("alice", "?window=7d").await["window"]["seconds"],
        7 * 86_400
    );
    // Windows longer than the decision retention (90 days) are clamped to it.
    assert_eq!(
        r.stats("alice", "?window=9999d").await["window"]["seconds"],
        90 * 86_400
    );
    for bad in [
        "?window=soon",
        "?window=0h",
        "?window=5w",
        "?group_by=nope",
        "?bogus=1",
    ] {
        let resp = reqwest::Client::new()
            .get(format!("{}/v1/stats{bad}", r.url))
            .bearer_auth(&r.keys["alice"])
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400, "{bad}");
    }
    assert_eq!(
        reqwest::get(format!("{}/v1/stats", r.url))
            .await
            .unwrap()
            .status(),
        401
    );
}

#[tokio::test]
async fn explain_shows_how_each_target_has_behaved_recently() {
    let r = rig().await;
    r.first.status.store(503, Ordering::SeqCst);
    for _ in 0..2 {
        r.ask("alice", "fallback").await;
    }
    let report: Value = reqwest::Client::new()
        .post(format!("{}/v1/route/explain", r.url))
        .bearer_auth(&r.keys["alice"])
        .json(&json!({"model": "fallback"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let targets = report["targets"].as_array().unwrap();
    let a = targets.iter().find(|t| t["target"] == "a").unwrap();
    assert_eq!(a["recent"]["attempts"], 2);
    assert_eq!(a["recent"]["failure_rate"], 1.0);
    assert_eq!(a["recent"]["window_hours"], 24);
    let b = targets.iter().find(|t| t["target"] == "b").unwrap();
    assert_eq!(b["recent"]["failure_rate"], 0.0);
    // A target with no traffic has no recent numbers; explain itself still works.
    let quiet: Value = reqwest::Client::new()
        .post(format!("{}/v1/route/explain", r.url))
        .bearer_auth(&r.keys["bob"])
        .json(&json!({"model": "fallback"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(quiet["outcome"], "ok");
    assert_eq!(quiet["targets"][0]["recent"], Value::Null);
}

// ---- ledger level: exact numbers ----

fn attempt(target: &str, outcome: &str, ms: u64) -> Attempt {
    Attempt {
        target: target.into(),
        provider: "p".into(),
        model: "m".into(),
        outcome: outcome.into(),
        ms,
        class: None,
        retry_after_ms: None,
        message: None,
    }
}

fn decision(id: &str, ts_ms: i64, route: &str, attempts: Vec<Attempt>) -> Decision {
    Decision {
        request_id: id.into(),
        ts_ms,
        key_id: Some("k".into()),
        route: Some(route.into()),
        attempts,
        outcome: "ok".into(),
        status: 200,
        ..Decision::default()
    }
}

#[tokio::test]
async fn latency_percentiles_match_known_values_and_old_decisions_fall_outside_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(&dir.path().join("l.db")).await.unwrap();
    let now = 1_800_000_000_000_i64;
    for n in 1..=10_u64 {
        ledger.record_decision(decision(
            &format!("r{n}"),
            now - 1000,
            "route",
            vec![attempt("t", "ok", n * 10)],
        ));
    }
    ledger.record_decision(decision(
        "old",
        now - 10 * 3_600_000,
        "route",
        vec![attempt("t", "ok", 99_999)],
    ));
    ledger.flush().await;
    let query = |since_ms| StatsQuery {
        key: Some("k".into()),
        since_ms,
        group_by: GroupBy::Target,
        route: None,
        target: None,
        row_cap: 1000,
    };
    let stats = ledger.stats(&query(now - 3_600_000)).await.unwrap();
    let t = &stats.groups[0];
    assert_eq!(
        (t.attempts, t.p50_ms, t.p90_ms, t.p99_ms),
        (10, Some(50), Some(90), Some(100))
    );
    assert!(!stats.truncated);
    // A wider window includes the old, slow attempt, and the tail moves.
    let wide = ledger.stats(&query(now - 20 * 3_600_000)).await.unwrap();
    assert_eq!(wide.groups[0].attempts, 11);
    assert_eq!(wide.groups[0].p99_ms, Some(99_999));
    // The same numbers by the public helper.
    let mut all: Vec<u64> = (1..=10).map(|n| n * 10).collect();
    all.push(99_999);
    assert_eq!(percentile(&all, 99), Some(99_999));

    // A row cap smaller than the window summarizes the newest decisions and says so.
    let capped = ledger
        .stats(&StatsQuery {
            row_cap: 3,
            ..query(now - 20 * 3_600_000)
        })
        .await
        .unwrap();
    assert!(capped.truncated);
    assert_eq!(capped.groups[0].attempts, 3);
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn statistics_are_read_only_and_a_broken_ledger_fails_the_read_not_requests() {
    let r = rig().await;
    r.ask("alice", "plain").await;
    // The ledger writes asynchronously: wait until the request's usage row is stored, so the
    // counts below compare steady states, not a write in flight.
    let count = || async {
        Ledger::open(&r.db)
            .await
            .unwrap()
            .entries_since(0)
            .await
            .unwrap()
            .len()
    };
    let mut before = 0;
    for _ in 0..100 {
        before = count().await;
        if before >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(before, 1, "the request's usage row was never written");
    for _ in 0..3 {
        r.stats("alice", "?group_by=endpoint").await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(count().await, before, "reading statistics writes nothing");

    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", r.db.display()))
        .await
        .unwrap();
    sqlx::query("DROP TABLE decisions")
        .execute(&pool)
        .await
        .unwrap();
    let resp = reqwest::Client::new()
        .get(format!("{}/v1/stats", r.url))
        .bearer_auth(&r.keys["alice"])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 500, "the read fails visibly");
    assert_eq!(
        r.ask("alice", "plain").await,
        200,
        "requests are unaffected"
    );
}
