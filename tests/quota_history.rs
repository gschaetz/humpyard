//! Quota history (ADR 0015, step 2): which polls are kept, what the ledger stores, and the
//! `GET /v1/quota/history` view that sets a provider's percent beside the cost of the calls made.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use common::{chat_completion, chat_request, serve, usage};
use humpyard::config::Config;
use humpyard::ledger::{Entry, Kind, Ledger, QuotaQuery, QuotaSample};
use humpyard::quota::{SAMPLE_EVERY_MS, Sampler, Window};
use humpyard::server;
use parking_lot::Mutex;
use serde_json::Value;

fn window(name: &str, percent: f64) -> Window {
    Window {
        name: name.into(),
        percent,
        status: "ok".into(),
        resets_at: Some("2026-10-23T23:37:14.000Z".into()),
        clamped: false,
    }
}

// ---- the sampling rule ----

#[test]
fn the_first_poll_is_kept_and_an_unchanged_one_is_not() {
    let mut sampler = Sampler::default();
    let windows = [window("monthly", 11.0)];
    assert!(sampler.due("p", &windows, 1_000));
    assert!(!sampler.due("p", &windows, 61_000), "nothing changed");
    assert!(!sampler.due("p", &windows, 1_000 + SAMPLE_EVERY_MS - 1));
}

#[test]
fn a_change_in_percent_status_or_reset_is_kept() {
    let mut sampler = Sampler::default();
    assert!(sampler.due("p", &[window("monthly", 11.0)], 0));
    assert!(sampler.due("p", &[window("monthly", 12.0)], 1));
    let mut limited = window("monthly", 12.0);
    limited.status = "limited".into();
    assert!(sampler.due("p", std::slice::from_ref(&limited), 2));
    limited.resets_at = Some("2026-11-23T23:37:14.000Z".into());
    assert!(sampler.due("p", &[limited], 3));
    assert!(
        sampler.due("p", &[window("monthly", 12.0), window("weekly", 1.0)], 4),
        "a window appearing is a change"
    );
}

#[test]
fn a_quiet_provider_is_still_kept_every_so_often_and_providers_are_independent() {
    let mut sampler = Sampler::default();
    let windows = [window("monthly", 11.0)];
    assert!(sampler.due("a", &windows, 0));
    assert!(
        sampler.due("b", &windows, 10),
        "another provider has its own history"
    );
    assert!(
        sampler.due("a", &windows, SAMPLE_EVERY_MS),
        "a heartbeat after the interval"
    );
    assert!(
        !sampler.due("a", &windows, SAMPLE_EVERY_MS + 1),
        "and the interval restarts"
    );
}

// ---- the ledger ----

/// A recent base time: the ledger prunes samples older than its retention (90 days by default),
/// so test samples must not be dated 1970.
fn base() -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    i64::try_from(now.as_millis()).unwrap() - 60_000
}

fn sample(ts_ms: i64, window: &str, percent: f64) -> QuotaSample {
    QuotaSample {
        ts_ms,
        provider: "p".into(),
        window: window.into(),
        percent,
        status: "ok".into(),
        resets_at: None,
    }
}

fn query(since_ms: i64) -> QuotaQuery {
    QuotaQuery {
        since_ms,
        provider: None,
        window: None,
        row_cap: 100,
    }
}

#[tokio::test]
async fn samples_are_stored_in_order_and_can_be_filtered() {
    let b = base();
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(&dir.path().join("l.db")).await.unwrap();
    ledger.record_quota(vec![
        sample(b + 1_000, "monthly", 12.0),
        sample(b + 1_000, "weekly", 1.0),
    ]);
    ledger.record_quota(vec![sample(b, "monthly", 11.0)]);
    ledger.record_quota(Vec::new()); // nothing to store is not a message
    ledger.flush().await;

    let all = ledger.quota_history(&query(0)).await.unwrap();
    let order: Vec<_> = all
        .iter()
        .map(|p| {
            (
                p.sample.ts_ms - b,
                p.sample.window.as_str(),
                p.sample.percent,
            )
        })
        .collect();
    assert_eq!(
        order,
        [
            (0, "monthly", 11.0),
            (1_000, "monthly", 12.0),
            (1_000, "weekly", 1.0)
        ]
    );

    let mut only_weekly = query(0);
    only_weekly.window = Some("weekly".into());
    assert_eq!(ledger.quota_history(&only_weekly).await.unwrap().len(), 1);
    let mut other = query(0);
    other.provider = Some("nobody".into());
    assert_eq!(ledger.quota_history(&other).await.unwrap().len(), 0);
    assert_eq!(
        ledger.quota_history(&query(b + 500)).await.unwrap().len(),
        2,
        "since is inclusive"
    );

    let mut capped = query(0);
    capped.row_cap = 2;
    let newest: Vec<_> = ledger
        .quota_history(&capped)
        .await
        .unwrap()
        .iter()
        .map(|p| p.sample.ts_ms - b)
        .collect();
    assert_eq!(
        newest,
        [1_000, 1_000],
        "a cap keeps the newest, still oldest first"
    );
}

fn call(ts_ms: i64, provider: &str, cost_micro_usd: u64) -> Entry {
    Entry {
        ts_ms,
        key_id: None,
        session_id: None,
        route: "r".into(),
        target: "t".into(),
        provider: provider.into(),
        model: "m".into(),
        kind: Kind::Answer,
        input_tokens: 0,
        cached_input_tokens: 0,
        cache_creation_tokens: 0,
        output_tokens: 0,
        reasoning_tokens: 0,
        cost_micro_usd,
        outcome: "ok".into(),
        usage_missing: false,
        request_id: None,
        signals: None,
    }
}

#[tokio::test]
async fn each_sample_counts_only_the_calls_between_it_and_the_previous_one() {
    let b = base();
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(&dir.path().join("l.db")).await.unwrap();
    ledger.record_quota(vec![sample(b, "monthly", 10.0), sample(b, "weekly", 1.0)]);
    ledger.record_quota(vec![sample(b + 1_000, "monthly", 11.0)]);
    ledger.record_quota(vec![sample(b + 3_000, "monthly", 12.0)]);
    for (at, provider, cost) in [
        (b - 500, "p", 1),         // before the first sample: belongs to no interval
        (b + 500, "p", 20),        // in the first interval
        (b + 500, "other", 4_000), // another provider's call is not this provider's cost
        (b + 1_000, "p", 300),     // exactly at a sample: counts toward that sample's interval
        (b + 2_000, "p", 4_000),   // in the second interval
        (b + 3_500, "p", 50_000),  // after the last sample: not yet counted
    ] {
        ledger.record(call(at, provider, cost));
    }
    ledger.flush().await;

    let mut monthly = query(0);
    monthly.window = Some("monthly".into());
    let costs: Vec<_> = ledger
        .quota_history(&monthly)
        .await
        .unwrap()
        .iter()
        .map(|p| p.cost_micro_usd_since_previous)
        .collect();
    assert_eq!(costs, [None, Some(320), Some(4_000)]);

    // Each window measures from its own previous sample.
    let mut weekly = query(0);
    weekly.window = Some("weekly".into());
    let only = ledger.quota_history(&weekly).await.unwrap();
    assert_eq!(only[0].cost_micro_usd_since_previous, None);
}

#[tokio::test]
async fn samples_past_the_retention_are_pruned() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open_with(&dir.path().join("l.db"), 1_024, 1)
        .await
        .unwrap();
    let b = base();
    ledger.record_quota(vec![
        sample(b - 3 * 86_400_000, "monthly", 1.0),
        sample(b, "monthly", 2.0),
    ]);
    ledger.flush().await;
    // The writer prunes after a batch; give it a moment, then only the recent sample remains.
    for _ in 0..100 {
        if ledger.quota_history(&query(0)).await.unwrap().len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let left = ledger.quota_history(&query(0)).await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].sample.percent, 2.0);
}

#[tokio::test]
async fn an_older_database_is_upgraded_in_place_and_keeps_its_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    let b = base();
    {
        let ledger = Ledger::open(&path).await.unwrap();
        ledger.record_quota(vec![sample(b, "monthly", 5.0)]);
        ledger.flush().await;
        // Roll the file back to the previous schema: no samples table, version 2.
        sqlx::query("DROP TABLE quota_samples")
            .execute(ledger.pool())
            .await
            .unwrap();
        sqlx::query("PRAGMA user_version = 2")
            .execute(ledger.pool())
            .await
            .unwrap();
        ledger.shutdown().await;
    }
    let ledger = Ledger::open(&path).await.unwrap();
    assert_eq!(ledger.quota_history(&query(0)).await.unwrap().len(), 0);
    ledger.record_quota(vec![sample(b + 1, "monthly", 6.0)]);
    ledger.flush().await;
    assert_eq!(ledger.quota_history(&query(0)).await.unwrap().len(), 1);
}

// ---- a running gateway ----

struct Source {
    doc: Mutex<String>,
}

fn doc(monthly: u32) -> String {
    format!(
        r#"{{"usage":{{"rolling":{{"status":"ok","percent":0,"resetsAt":"2026-10-13T15:00:05.000Z"}},"monthly":{{"status":"ok","percent":{monthly},"resetsAt":"2026-10-23T23:37:14.000Z"}}}}}}"#
    )
}

async fn usage_doc(State(s): State<Arc<Source>>) -> impl IntoResponse {
    ([("content-type", "application/json")], s.doc.lock().clone())
}

async fn completions() -> impl IntoResponse {
    axum::Json(chat_completion("m", "served", Some(usage(10, 0))))
}

struct Gateway {
    url: String,
    key: String,
    src: Arc<Source>,
    _dir: tempfile::TempDir,
}

async fn gateway(with_ledger: bool) -> Gateway {
    let src = Arc::new(Source {
        doc: Mutex::new(doc(11)),
    });
    let app = Router::new()
        .route("/usage", get(usage_doc))
        .route("/chat/completions", post(completions))
        .with_state(src.clone());
    let base = format!("http://{}", serve(app).await);
    let dir = tempfile::tempdir().unwrap();
    let ledger = if with_ledger {
        format!(
            "[ledger]\npath = \"{}\"\n",
            dir.path().join("l.db").display()
        )
    } else {
        String::new()
    };
    let (key, hash) = humpyard::auth::generate_key().unwrap();
    // Ten input tokens at 1,000,000 USD per million tokens cost 10,000,000 micro-USD.
    let text = format!(
        r#"listen = "127.0.0.1:0"
quota_poll_secs = 1
quota_stale_secs = 60
{ledger}
[keys.k]
sha256 = "{hash}"
[providers.p]
base_url = "{base}"
api_key_env = "K"
quota_source = "opencode-go"
max_retries = 0
[targets.t]
endpoints = [{{ provider = "p", model = "m", price = {{ input = 1000000.0, output = 0.0 }} }}]
"#
    );
    let config = Config::from_toml(&text, |_| Some("secret".into())).unwrap();
    Gateway {
        url: format!(
            "http://{}",
            serve(server::router(config).await.unwrap()).await
        ),
        key,
        src,
        _dir: dir,
    }
}

impl Gateway {
    async fn get(&self, path: &str, key: Option<&str>) -> reqwest::Response {
        let mut request = reqwest::Client::new().get(format!("{}{path}", self.url));
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        request.send().await.unwrap()
    }

    async fn history(&self, query: &str) -> Value {
        self.get(&format!("/v1/quota/history{query}"), Some(&self.key))
            .await
            .json()
            .await
            .unwrap()
    }

    /// Reads history until `done` accepts its data, or fails the test.
    async fn wait_for(&self, what: &str, done: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        for _ in 0..200 {
            let data = self.history("").await["data"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if done(&data) {
                return data;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("never saw: {what}; last: {}", self.history("").await);
    }
}

#[tokio::test]
async fn history_sets_a_windows_change_beside_the_cost_of_the_calls_made() {
    let gw = gateway(true).await;
    // The first poll is stored: one sample per window.
    let first = gw.wait_for("the first samples", |d| d.len() >= 2).await;
    assert_eq!(
        first.len(),
        2,
        "an unchanged source is not stored again: {first:?}"
    );
    assert!(first.iter().all(|p| p["provider"] == "p"));
    assert!(
        first
            .iter()
            .all(|p| p["cost_micro_usd_since_previous"].is_null()),
        "no earlier sample to measure from"
    );

    // One priced call, then the source moves the monthly window.
    let reply = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", gw.url))
        .bearer_auth(&gw.key)
        .json(&chat_request("t", false))
        .send()
        .await
        .unwrap();
    assert_eq!(reply.status(), 200);
    *gw.src.doc.lock() = doc(12);

    let data = gw
        .wait_for("a second monthly sample", |d| {
            d.iter().filter(|p| p["window"] == "monthly").count() >= 2
        })
        .await;
    let monthly: Vec<&Value> = data.iter().filter(|p| p["window"] == "monthly").collect();
    assert_eq!(monthly[0]["percent"], 11.0);
    assert_eq!(monthly[1]["percent"], 12.0);
    assert_eq!(monthly[1]["status"], "ok");
    assert_eq!(monthly[1]["resets_at"], "2026-10-23T23:37:14.000Z");
    assert_eq!(
        monthly[1]["cost_micro_usd_since_previous"], 10_000_000,
        "the call made between the two samples"
    );
    // A poll worth keeping is stored whole, so the unmoved rolling window has two samples too.
    assert_eq!(data.iter().filter(|p| p["window"] == "rolling").count(), 2);

    // Filters and the response envelope.
    let body = gw.history("?name=rolling&provider=p&window=1h").await;
    let rolling = body["data"].as_array().unwrap().len();
    assert_eq!(rolling, 2);
    assert_eq!(body["object"], "quota_history");
    assert_eq!(body["window"]["seconds"], 3600);
    assert!(
        !body.to_string().contains("secret"),
        "no credential in history"
    );
}

#[tokio::test]
async fn history_needs_a_valid_key_a_ledger_and_known_parameters() {
    let gw = gateway(true).await;
    assert_eq!(gw.get("/v1/quota/history", None).await.status(), 401);
    assert_eq!(
        gw.get("/v1/quota/history", Some("sk-wrong")).await.status(),
        401
    );
    for bad in ["?window=soon", "?window=0h", "?surprise=1"] {
        let r = gw
            .get(&format!("/v1/quota/history{bad}"), Some(&gw.key))
            .await;
        assert_eq!(r.status(), 400, "{bad}");
    }

    let plain = gateway(false).await;
    let r = plain.get("/v1/quota/history", Some(&plain.key)).await;
    assert_eq!(r.status(), 404);
    let text = r.text().await.unwrap();
    assert!(text.contains("no ledger"), "{text}");
}
