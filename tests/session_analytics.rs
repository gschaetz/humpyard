//! Session and cache analytics: cache numbers on every stats group, `group_by=session`, and the switch
//! report. All read-only views over a seeded ledger; per-key isolation, bounds and the upgrade included.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use common::serve;
use humpyard::clock::{Clock, SystemClock};
use humpyard::config::Config;
use humpyard::ledger::{Attempt, Decision, Entry, Kind, Ledger, SessionQuery};
use humpyard::server;
use serde_json::{Value, json};

struct Rig {
    url: String,
    alice: String,
    bob: String,
    _dir: tempfile::TempDir,
}

/// One answered usage row. Times count down from now so the rows sit inside any window.
#[allow(clippy::too_many_arguments)]
fn call(
    key: &str,
    session: Option<&str>,
    age_s: i64,
    provider: &str,
    model: &str,
    tokens: (u64, u64, u64, u64), // fresh input, cached input, cache creation, output
    cost: u64,
) -> Entry {
    Entry {
        ts_ms: SystemClock.now_ms() - age_s * 1000,
        key_id: Some(key.into()),
        session_id: session.map(str::to_string),
        route: "r".into(),
        target: format!("t-{provider}"),
        provider: provider.into(),
        model: model.into(),
        kind: Kind::Answer,
        input_tokens: tokens.0,
        cached_input_tokens: tokens.1,
        cache_creation_tokens: tokens.2,
        output_tokens: tokens.3,
        reasoning_tokens: 0,
        cost_micro_usd: cost,
        outcome: "ok".into(),
        usage_missing: false,
        request_id: None,
    }
}

/// A decision for the same request, so the existing statistics groups exist.
fn decision(key: &str, target: &str, provider: &str, model: &str, age_s: i64) -> Decision {
    Decision {
        request_id: format!("req-{age_s}-{provider}"),
        ts_ms: SystemClock.now_ms() - age_s * 1000,
        key_id: Some(key.into()),
        format: "openai_chat".into(),
        route: Some("r".into()),
        served_target: Some(target.into()),
        served_provider: Some(provider.into()),
        status: 200,
        outcome: "ok".into(),
        attempts: vec![Attempt {
            target: target.into(),
            provider: provider.into(),
            model: model.into(),
            outcome: "ok".into(),
            ms: 10,
            class: None,
            retry_after_ms: None,
            message: None,
        }],
        ..Decision::default()
    }
}

/// The rows and decisions the tests read (see `rig`).
fn seed(ledger: &Ledger) {
    let a = ("p1", "m-a");
    let b = ("p2", "m-b");
    let c = ("p3", "m-c");
    let rows = vec![
        // stay: a, a, a (cache written, then read twice)
        call(
            "alice",
            Some("stay"),
            600,
            a.0,
            a.1,
            (100, 0, 900, 50),
            1_000,
        ),
        call("alice", Some("stay"), 590, a.0, a.1, (50, 900, 0, 20), 400),
        call("alice", Some("stay"), 580, a.0, a.1, (40, 950, 0, 20), 380),
        // switch: a, a, b (the second call left 900 tokens cached; b starts cold)
        call(
            "alice",
            Some("switch"),
            500,
            a.0,
            a.1,
            (100, 0, 900, 50),
            1_000,
        ),
        call(
            "alice",
            Some("switch"),
            490,
            a.0,
            a.1,
            (50, 900, 0, 20),
            400,
        ),
        call(
            "alice",
            Some("switch"),
            480,
            b.0,
            b.1,
            (1_000, 0, 0, 30),
            2_000,
        ),
        // twice: a, b, a (the b call left 200 cached, the first a call 300)
        call("alice", Some("twice"), 400, a.0, a.1, (10, 300, 0, 5), 100),
        call("alice", Some("twice"), 390, b.0, b.1, (10, 200, 0, 5), 80),
        call("alice", Some("twice"), 380, a.0, a.1, (500, 0, 0, 5), 900),
        // unpriced: a, then c (no price): tokens counted, no cost estimate
        call(
            "alice",
            Some("unpriced"),
            300,
            a.0,
            a.1,
            (10, 400, 0, 5),
            120,
        ),
        call(
            "alice",
            Some("unpriced"),
            290,
            c.0,
            c.1,
            (500, 0, 0, 5),
            300,
        ),
        // no session id
        call("alice", None, 200, a.0, a.1, (10, 0, 0, 5), 50),
        call("alice", None, 190, a.0, a.1, (10, 0, 0, 5), 50),
        // bob's session
        call("bob", Some("b1"), 100, a.0, a.1, (10, 90, 0, 5), 60),
    ];
    for row in rows {
        ledger.record(row);
    }
    // A judge call (a routing-time classifier) and a call whose provider reported no usage.
    let mut judge = call("alice", Some("stay"), 570, a.0, a.1, (7, 0, 0, 7), 7);
    judge.kind = Kind::Judge;
    ledger.record(judge);
    let mut missing = call("alice", Some("stay"), 560, a.0, a.1, (0, 0, 0, 0), 0);
    missing.usage_missing = true;
    ledger.record(missing);
    // Decisions, so the target statistics groups exist.
    ledger.record_decision(decision("alice", "t-p1", a.0, a.1, 100));
    ledger.record_decision(decision("alice", "t-p2", b.0, b.1, 101));
}

/// Seeds a ledger, closes it, and serves a gateway over it. Endpoints: `p1/m-a` (priced 3.0 fresh, 0.3
/// cached), `p2/m-b` (priced 1.0 and 0.1) and `p3/m-c` (no price).
///
/// alice: session `stay` (3 calls on p1/m-a, cache warming), `switch` (p1/m-a, p1/m-a, then p2/m-b),
/// `twice` (a, b, a), `unpriced` (a, then p3/m-c), plus two answered calls with no session id, a judge
/// call and a call with missing usage. bob: session `b1`.
async fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ledger.db");
    let ledger = Ledger::open(&db).await.unwrap();
    seed(&ledger);
    ledger.flush().await;
    ledger.shutdown().await;

    let (alice, alice_hash) = humpyard::auth::generate_key().unwrap();
    let (bob, bob_hash) = humpyard::auth::generate_key().unwrap();
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[ledger]
path = "{db}"
[keys.alice]
sha256 = "{alice_hash}"
[keys.bob]
sha256 = "{bob_hash}"
[providers.p1]
base_url = "http://127.0.0.1:1"
api_key_env = "K"
[providers.p2]
base_url = "http://127.0.0.1:1"
api_key_env = "K"
[providers.p3]
base_url = "http://127.0.0.1:1"
api_key_env = "K"
[targets.t-p1]
endpoints = [{{ provider = "p1", model = "m-a", price = {{ input = 3.0, output = 6.0, cached_input = 0.3 }} }}]
[targets.t-p2]
endpoints = [{{ provider = "p2", model = "m-b", price = {{ input = 1.0, output = 2.0, cached_input = 0.1 }} }}]
[targets.t-p3]
endpoints = [{{ provider = "p3", model = "m-c" }}]
"#,
        db = db.display(),
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    Rig {
        url: format!(
            "http://{}",
            serve(server::router(config).await.unwrap()).await
        ),
        alice,
        bob,
        _dir: dir,
    }
}

impl Rig {
    async fn get(&self, key: &str, path: &str) -> (u16, Value) {
        let resp = reqwest::Client::new()
            .get(format!("{}{path}", self.url))
            .bearer_auth(key)
            .send()
            .await
            .unwrap();
        (
            resp.status().as_u16(),
            resp.json().await.unwrap_or(Value::Null),
        )
    }
}

fn row<'a>(body: &'a Value, group: &str) -> &'a Value {
    body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["group"] == group)
        .unwrap_or_else(|| panic!("no group {group} in {body}"))
}

#[tokio::test]
async fn cache_numbers_are_on_every_existing_stats_group() {
    let rig = rig().await;
    let (status, body) = rig
        .get(&rig.alice, "/v1/stats?group_by=target&window=1h")
        .await;
    assert_eq!(status, 200, "{body}");
    // Alice's answered calls on t-p1 that reported usage: stay (3), switch (2), twice (2), unpriced (1),
    // and the two without a session. The judge and the missing-usage row are not answers with usage.
    let t = &row(&body, "t-p1")["tokens"];
    assert_eq!(
        t["input"],
        100 + 50 + 40 + 100 + 50 + 10 + 500 + 10 + 10 + 10
    );
    assert_eq!(t["cached_input"], 900 + 950 + 900 + 300 + 400);
    assert_eq!(t["cache_creation"], 900 + 900);
    let ratio = row(&body, "t-p1")["cache_hit_ratio"].as_f64().unwrap();
    let prompt = 880 + 3_450 + 1_800; // fresh + cached + cache creation
    assert!(
        (ratio - 3_450.0 / f64::from(prompt)).abs() < 1e-9,
        "{ratio}"
    );
    // An unchanged field is still there.
    assert!(row(&body, "t-p1")["avg_cost_usd"].is_number());
}

#[tokio::test]
async fn sessions_are_listed_with_cost_cache_endpoints_and_switches() {
    let rig = rig().await;
    let (status, body) = rig
        .get(&rig.alice, "/v1/stats?group_by=session&window=1h")
        .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["group_by"], "session");
    assert_eq!(body["sessions_total"], 4);
    assert_eq!(body["answered_without_session"], 2);
    // Costliest first: switch (3400), stay (1780), twice (1080), unpriced (420).
    let order: Vec<&str> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["group"].as_str().unwrap())
        .collect();
    assert_eq!(order, vec!["switch", "stay", "twice", "unpriced"]);

    let switch = row(&body, "switch");
    assert_eq!(switch["requests"], 3);
    assert_eq!(switch["cost_micro_usd"], 3_400);
    assert_eq!(switch["switches"], 1);
    assert_eq!(switch["endpoints"], json!(["p1/m-a", "p2/m-b"]));
    assert_eq!(switch["tokens"]["cached_input"], 900);
    // The stay session has a judge row and a missing-usage row that must not change its figures.
    let stay = row(&body, "stay");
    assert_eq!(
        stay["requests"], 4,
        "answered calls, including the one with no usage"
    );
    assert_eq!(stay["switches"], 0);
    assert_eq!(stay["tokens"]["input"], 190);
    assert_eq!(stay["tokens"]["cached_input"], 1_850);
    assert_eq!(row(&body, "twice")["switches"], 2);
}

#[tokio::test]
async fn limit_and_order_by_shape_the_list_and_bad_parameters_are_refused() {
    let rig = rig().await;
    let (_, body) = rig
        .get(
            &rig.alice,
            "/v1/stats?group_by=session&window=1h&order_by=switches&limit=2",
        )
        .await;
    let order: Vec<&str> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["group"].as_str().unwrap())
        .collect();
    assert_eq!(
        order,
        vec!["twice", "switch"],
        "most switches first, two rows"
    );
    assert_eq!(body["sessions_total"], 4, "the total is not limited");

    let (_, recent) = rig
        .get(
            &rig.alice,
            "/v1/stats?group_by=session&window=1h&order_by=recent&limit=1",
        )
        .await;
    assert_eq!(
        recent["data"][0]["group"], "unpriced",
        "its last call is the newest"
    );

    for query in [
        "limit=0",
        "limit=101",
        "limit=abc",
        "order_by=colour",
        "route=r",
    ] {
        let (status, body) = rig
            .get(&rig.alice, &format!("/v1/stats?group_by=session&{query}"))
            .await;
        assert_eq!(status, 400, "{query}: {body}");
    }
}

#[tokio::test]
async fn a_key_sees_only_its_own_sessions() {
    let rig = rig().await;
    let (_, bob) = rig
        .get(&rig.bob, "/v1/stats?group_by=session&window=1h")
        .await;
    assert_eq!(bob["sessions_total"], 1);
    assert_eq!(bob["data"][0]["group"], "b1");
    let (_, switches) = rig.get(&rig.bob, "/v1/stats/switches?window=1h").await;
    assert_eq!(switches["sessions"]["total"], 1);
    assert_eq!(switches["switches"]["count"], 0);
}

#[tokio::test]
async fn the_switch_report_counts_abandoned_tokens_and_prices_them_from_the_config() {
    let rig = rig().await;
    let (status, body) = rig.get(&rig.alice, "/v1/stats/switches?window=1h").await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["sessions"], json!({"total": 4, "switched": 3}));
    // switch: a->b after 900 warm; twice: a->b after 300 warm, b->a after 200; unpriced: a->c after 400.
    assert_eq!(body["switches"]["count"], 4);
    assert_eq!(
        body["switches"]["warm_tokens_abandoned"],
        900 + 300 + 200 + 400
    );
    // Priced at the destination's fresh minus cached input price (per million tokens = micro-USD per
    // token): a->b 0.9 x 900 + 0.9 x 300 = 1,080; b->a 2.7 x 200 = 540; a->c has no price.
    assert_eq!(body["switches"]["priced"], 3);
    assert_eq!(body["switches"]["unpriced"], 1);
    let extra = body["switches"]["estimated_extra_cost_usd"]
        .as_f64()
        .unwrap();
    assert!((extra - 0.001_620).abs() < 1e-12, "{extra}");
    let total = body["spend"]["total_usd"].as_f64().unwrap();
    assert!(
        (body["spend"]["extra_share"].as_f64().unwrap() - extra / total).abs() < 1e-12,
        "{body}"
    );
    // The pairs that switch most.
    let pairs = body["by_pair"].as_array().unwrap();
    assert_eq!(pairs[0]["from"], "p1/m-a");
    assert_eq!(pairs[0]["to"], "p2/m-b");
    assert_eq!(pairs[0]["count"], 2);
    assert_eq!(pairs[0]["warm_tokens_abandoned"], 1_200);
    // Stayed versus switched, with the caveat.
    assert_eq!(body["stayed"]["sessions"], 1);
    assert_eq!(body["switched"]["sessions"], 3);
    assert!(body["note"].as_str().unwrap().contains("not causal"));
}

#[tokio::test]
async fn coverage_says_how_many_answered_calls_carried_a_session_id() {
    let rig = rig().await;
    let (_, body) = rig.get(&rig.alice, "/v1/stats/switches?window=1h").await;
    // 14 answers for alice (the judge row is not an answer): 12 with a session id and 2 without. The
    // missing-usage row is an answered call with a session id, so it counts as covered.
    assert_eq!(body["coverage"]["answered"], 14);
    assert_eq!(body["coverage"]["with_session_id"], 12);
    let ratio = body["coverage"]["ratio"].as_f64().unwrap();
    assert!((ratio - 12.0 / 14.0).abs() < 1e-9, "{ratio}");
}

async fn scan(ledger: &Ledger, row_cap: u32) -> humpyard::ledger::SessionScan {
    ledger
        .session_scan(&SessionQuery {
            key: Some("k".into()),
            since_ms: 0,
            row_cap,
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn a_scan_past_the_row_cap_is_marked_truncated() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(&dir.path().join("l.db")).await.unwrap();
    for n in 0..5 {
        ledger.record(call("k", Some("s"), 100 - n, "p", "m", (1, 0, 0, 1), 1));
    }
    ledger.flush().await;
    let capped = scan(&ledger, 3).await;
    assert!(capped.truncated);
    assert_eq!(capped.sessions[0].calls, 3, "the newest three rows");
    let full = scan(&ledger, 10).await;
    assert!(!full.truncated);
    assert_eq!(full.sessions[0].calls, 5);
}

#[tokio::test]
async fn the_session_views_need_a_ledger() {
    let (key, hash) = humpyard::auth::generate_key().unwrap();
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[keys.alice]
sha256 = "{hash}"
[providers.p]
base_url = "http://127.0.0.1:1"
api_key_env = "K"
[targets.t]
endpoints = [{{ provider = "p", model = "m" }}]
"#
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    for path in ["/v1/stats?group_by=session", "/v1/stats/switches"] {
        let status = reqwest::Client::new()
            .get(format!("{url}{path}"))
            .bearer_auth(&key)
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(status, 404, "{path}");
    }
}

#[tokio::test]
async fn an_older_ledger_gets_the_session_index_and_keeps_its_rows() {
    use sqlx::Connection;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    let ledger = Ledger::open(&path).await.unwrap();
    ledger.record(call("k", Some("s"), 10, "p", "m", (5, 0, 0, 5), 9));
    ledger.flush().await;
    ledger.shutdown().await;
    // A version-6 file: no session index. Built offline so no live pool holds a stale statement.
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&path);
    let mut conn = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    for statement in ["DROP INDEX usage_session_ts", "PRAGMA user_version = 6"] {
        sqlx::query(statement).execute(&mut conn).await.unwrap();
    }
    conn.close().await.unwrap();

    let ledger = Ledger::open(&path).await.unwrap();
    let scan = ledger
        .session_scan(&SessionQuery {
            key: Some("k".into()),
            since_ms: 0,
            row_cap: 100,
        })
        .await
        .unwrap();
    assert_eq!(scan.sessions.len(), 1, "the earlier row is intact");
    let mut conn = sqlx::SqliteConnection::connect_with(&options.read_only(true))
        .await
        .unwrap();
    let indexes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'usage_session_ts'",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap();
    assert_eq!(indexes, 1, "the upgrade created the index");
}
