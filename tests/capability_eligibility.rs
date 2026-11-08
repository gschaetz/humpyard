//! Capability eligibility (ADR 0019): endpoints declare what they can take, a target that cannot serve a
//! request is removed before routing, only on certainty, and a request nothing can serve is refused with
//! its reason before any upstream call.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::post;
use common::{chat_completion, serve, usage};
use humpyard::clock::Clock;
use humpyard::config::Config;
use humpyard::estimate::{estimate_tokens, estimate_tokens_low};
use humpyard::server;
use serde_json::{Value, json};

struct Upstream {
    calls: AtomicUsize,
}

impl Upstream {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

async fn completions(State(up): State<Arc<Upstream>>) -> axum::response::Response {
    up.calls.fetch_add(1, Ordering::SeqCst);
    axum::Json(chat_completion("m", "ok", Some(usage(10, 5)))).into_response()
}

async fn upstream() -> (Arc<Upstream>, String) {
    let up = Arc::new(Upstream {
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
    small: Arc<Upstream>,
    big: Arc<Upstream>,
    plain: Arc<Upstream>,
    limited: Arc<Upstream>,
    capped: Arc<Upstream>,
    db: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

/// Targets: `small` (window 1000 tokens, no tools, no images), `big` (window 1,000,000, tools and images),
/// `plain` (declares nothing), `mixed` (one endpoint with a window of 100 tokens and one of 1,000,000).
/// Routes: `ladder` (small, big), `only-small` (small), `open` (plain, big), `mixed-first` (mixed, big),
/// `strict-small` (small private, big public, only private allowed).
/// A per-request cap (`add-endpoint-request-cap`): `groq` declares a huge window and a cap of 100 tokens,
/// `mixed-cap` has a capped endpoint and an uncapped one. Routes: `cap-ladder` (groq, big), `only-groq`,
/// `cap-mixed` (mixed-cap, big).
fn cap_section(capped_url: &str) -> String {
    format!(
        r#"[providers.pg]
base_url = "{capped_url}"
api_key_env = "K"
max_retries = 0
[targets.groq]
endpoints = [{{ provider = "pg", model = "m", context_window = 1000000, max_request_tokens = 100 }}]
[targets.mixed-cap]
endpoints = [
  {{ provider = "pg", model = "m", max_request_tokens = 100 }},
  {{ provider = "pb", model = "m" }},
]
[routes.cap-ladder]
type = "passthrough"
targets = ["groq", "big"]
[routes.only-groq]
type = "passthrough"
targets = ["groq"]
[routes.cap-mixed]
type = "passthrough"
targets = ["mixed-cap", "big"]
"#
    )
}

async fn rig() -> Rig {
    let (small, small_url) = upstream().await;
    let (big, big_url) = upstream().await;
    let (plain, plain_url) = upstream().await;
    let (limited, limited_url) = upstream().await;
    let (capped, capped_url) = upstream().await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ledger.db");
    let (key, key_hash) = humpyard::auth::generate_key().unwrap();
    let (credential, credential_hash) = humpyard::auth::generate_key().unwrap();
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[ledger]
path = "{db}"
[metrics]
enabled = true
bearer_sha256 = "{credential_hash}"
[keys.alice]
sha256 = "{key_hash}"
[providers.ps]
base_url = "{small_url}"
api_key_env = "K"
max_retries = 0
[providers.pb]
base_url = "{big_url}"
api_key_env = "K"
max_retries = 0
[providers.pp]
base_url = "{plain_url}"
api_key_env = "K"
max_retries = 0
[providers.pl]
base_url = "{limited_url}"
api_key_env = "K"
max_retries = 0
{cap}
[targets.small]
class = "private"
endpoints = [{{ provider = "ps", model = "m", context_window = 1000, tools = false, vision = false }}]
[targets.big]
class = "public"
endpoints = [{{ provider = "pb", model = "m", context_window = 1000000, tools = true, vision = true }}]
[targets.plain]
endpoints = [{{ provider = "pp", model = "m" }}]
[targets.mixed]
endpoints = [
  {{ provider = "pl", model = "m", context_window = 100 }},
  {{ provider = "pb", model = "m", context_window = 1000000 }},
]
[routes.ladder]
type = "passthrough"
targets = ["small", "big"]
[routes.only-small]
type = "passthrough"
targets = ["small"]
[routes.open]
type = "passthrough"
targets = ["plain", "big"]
[routes.mixed-first]
type = "passthrough"
targets = ["mixed", "big"]
[routes.strict-small]
type = "passthrough"
targets = ["small", "big"]
allowed_classes = ["private"]
"#,
        db = db.display(),
        cap = cap_section(&capped_url),
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    Rig {
        url: format!(
            "http://{}",
            serve(server::router(config).await.unwrap()).await
        ),
        key,
        credential,
        small,
        big,
        plain,
        limited,
        capped,
        db,
        _dir: dir,
    }
}

fn plain_request(route: &str, text: &str) -> Value {
    json!({"model": route, "messages": [{"role": "user", "content": text}]})
}

fn tools_request(route: &str) -> Value {
    json!({
        "model": route,
        "messages": [{"role": "user", "content": "what is the weather"}],
        "tools": [{"type": "function", "function": {
            "name": "weather", "description": "get the weather",
            "parameters": {"type": "object", "properties": {"city": {"type": "string"}}}}}]
    })
}

fn image_request(route: &str) -> Value {
    json!({
        "model": route,
        "messages": [{"role": "user", "content": [
            {"type": "text", "text": "what is this"},
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,iVBORw0KGgo="}}
        ]}]
    })
}

impl Rig {
    /// Status, the serving target (empty on failure) and the JSON body.
    async fn send(&self, body: &Value) -> (u16, String, Value) {
        let resp = reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .bearer_auth(&self.key)
            .json(body)
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        let target = resp
            .headers()
            .get("x-humpyard-target")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        (status, target, resp.json().await.unwrap_or(Value::Null))
    }

    async fn explain(&self, body: &Value) -> Value {
        reqwest::Client::new()
            .post(format!("{}/v1/route/explain", self.url))
            .bearer_auth(&self.key)
            .json(body)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap()
    }

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
async fn a_request_with_tools_skips_a_target_that_declares_no_tools() {
    let rig = rig().await;
    let (status, target, _) = rig.send(&tools_request("ladder")).await;
    assert_eq!((status, target.as_str()), (200, "big"));
    assert_eq!(rig.small.calls(), 0, "the no-tools target is never called");
    let decision = rig.decision("ladder").await;
    assert_eq!(
        decision["incapable_targets"],
        json!([{"target": "small", "reason": "tools"}])
    );
}

#[tokio::test]
async fn a_request_with_images_skips_a_target_that_declares_no_vision() {
    let rig = rig().await;
    let (status, target, _) = rig.send(&image_request("ladder")).await;
    assert_eq!((status, target.as_str()), (200, "big"));
    assert_eq!(rig.small.calls(), 0);
    assert_eq!(
        rig.decision("ladder").await["incapable_targets"],
        json!([{"target": "small", "reason": "vision"}])
    );
}

#[tokio::test]
async fn an_oversized_prompt_skips_a_target_whose_window_it_certainly_exceeds() {
    let rig = rig().await;
    // 12,000 bytes is at least 2,000 tokens even at 6 bytes per token; small's window is 1,000.
    let (status, target, _) = rig
        .send(&plain_request("ladder", &"word ".repeat(2400)))
        .await;
    assert_eq!((status, target.as_str()), (200, "big"));
    assert_eq!(rig.small.calls(), 0);
    let text = rig.metrics().await;
    assert!(
        text.contains(
            r#"humpyard_capability_skips_total{route="ladder",target="small",reason="context"} 1"#
        ),
        "{text}"
    );
}

#[tokio::test]
async fn a_borderline_prompt_is_not_removed() {
    let rig = rig().await;
    // 4,500 bytes: about 1,500 tokens by the ordinary (high) estimate, which is over small's window, but
    // only 750 by the low bound, so a target that could have served is not removed.
    let (status, target, _) = rig
        .send(&plain_request("ladder", &"word ".repeat(900)))
        .await;
    assert_eq!((status, target.as_str()), (200, "small"));
    assert_eq!(rig.small.calls(), 1);
    assert_eq!(rig.decision("ladder").await["incapable_targets"], json!([]));
}

#[tokio::test]
async fn an_endpoint_that_declares_nothing_is_capable() {
    let rig = rig().await;
    let (status, target, _) = rig.send(&tools_request("open")).await;
    assert_eq!((status, target.as_str()), (200, "plain"));
    assert_eq!(rig.plain.calls(), 1);
}

#[tokio::test]
async fn a_target_stays_when_any_of_its_endpoints_can_serve() {
    let rig = rig().await;
    // The first endpoint's window (100 tokens) is exceeded, the second's is not: the target stays. The
    // pool still tries endpoints in order, so the limited one is asked first (endpoint-level skipping is
    // a later change) and answers here.
    let (status, target, _) = rig
        .send(&plain_request("mixed-first", &"word ".repeat(2400)))
        .await;
    assert_eq!((status, target.as_str()), (200, "mixed"));
    assert_eq!(rig.limited.calls(), 1);
    assert_eq!(
        rig.decision("mixed-first").await["incapable_targets"],
        json!([])
    );
}

#[tokio::test]
async fn a_request_nothing_can_serve_is_refused_before_any_upstream_call() {
    let rig = rig().await;
    let (status, _, body) = rig.send(&tools_request("only-small")).await;
    assert_eq!(status, 400);
    let message = body["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("no target of route `only-small`")
            && message.contains("tools = false")
            && message.contains("`small`"),
        "{message}"
    );
    assert_eq!(rig.small.calls(), 0, "refused before any upstream call");
    let decision = rig.decision("only-small").await;
    assert_eq!(decision["outcome"], "no_capable_target");
    assert_eq!(
        decision["incapable_targets"],
        json!([{"target": "small", "reason": "tools"}])
    );
    let text = rig.metrics().await;
    assert!(
        text.contains(r#"humpyard_refusals_total{reason="no_capable_target"} 1"#),
        "{text}"
    );
}

#[tokio::test]
async fn capability_never_adds_a_target_to_a_strict_route() {
    let rig = rig().await;
    // Only the private `small` is allowed; it cannot take tools, and the public `big` that could is not
    // added back.
    let (status, _, body) = rig.send(&tools_request("strict-small")).await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(rig.big.calls(), 0);
    assert_eq!(rig.small.calls(), 0);
}

#[tokio::test]
async fn explain_reports_what_capability_removes_and_when_it_would_refuse() {
    let rig = rig().await;
    let removed = rig
        .explain(&json!({"model": "ladder", "tools": true}))
        .await;
    assert_eq!(removed["outcome"], "ok");
    assert_eq!(
        removed["incapable_targets"],
        json!([{"target": "small", "reason": "tools"}])
    );
    let planned: Vec<&str> = removed["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["target"].as_str().unwrap())
        .collect();
    assert_eq!(planned, vec!["big"]);

    let refused = rig
        .explain(&json!({"model": "only-small", "images": true}))
        .await;
    assert_eq!(refused["outcome"], "no_capable_target");
    assert!(
        refused["message"]
            .as_str()
            .unwrap()
            .contains("vision = false")
    );

    let big_prompt = rig
        .explain(&json!({"model": "ladder", "prompt_tokens": 5000}))
        .await;
    assert_eq!(
        big_prompt["incapable_targets"],
        json!([{"target": "small", "reason": "context"}])
    );
}

/// The query `docs/routing.md` gives for tuning the low estimate must run against a real ledger.
#[tokio::test]
async fn the_documented_tuning_query_runs_on_a_real_ledger() {
    use sqlx::{Connection, Row};
    let rig = rig().await;
    // A declared window makes the gateway record the prompt estimate, so the query has rows to join.
    for _ in 0..2 {
        let (status, _, _) = rig
            .send(&plain_request("ladder", &"word ".repeat(100)))
            .await;
        assert_eq!(status, 200);
    }
    rig.decision("ladder").await; // the ledger writer is asynchronous
    tokio::time::sleep(Duration::from_millis(300)).await;
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&rig.db)
        .read_only(true);
    let mut conn = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    let docs =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/routing.md")).unwrap();
    // The query is the quoted argument of the `sqlite3` line (it contains no double quote).
    let start = docs.find("sqlite3 humpyard.db \"").unwrap() + "sqlite3 humpyard.db \"".len();
    let end = start + docs[start..].find('"').unwrap();
    let sql = docs[start..end].to_string();
    let row = sqlx::query(sqlx::AssertSqlSafe(sql))
        .fetch_one(&mut conn)
        .await
        .unwrap();
    let count: i64 = row.get(0);
    let worst: f64 = row.get(1);
    assert!(count >= 1, "the query joined {count} rows");
    assert!(worst > 0.0, "worst ratio {worst}");
}

#[test]
fn a_zero_window_is_rejected_and_an_undeclared_config_loads() {
    let config = |endpoint: &str| {
        format!(
            r#"listen = "127.0.0.1:8080"
[providers.p]
base_url = "http://localhost:1/v1"
api_key_env = "K"
[targets.a]
endpoints = [{{ provider = "p", model = "m"{endpoint} }}]
"#
        )
    };
    let load = |text: String| Config::from_toml(&text, |_| Some("k".into()));
    assert!(load(config("")).is_ok(), "declaring nothing is fine");
    assert!(
        load(config(
            ", context_window = 8000, tools = true, vision = false"
        ))
        .is_ok(),
        "all three declarations load"
    );
    let err = load(config(", context_window = 0"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("context_window must be at least 1"), "{err}");
}

#[test]
fn the_low_estimate_stays_below_known_token_counts_and_below_the_ordinary_estimate() {
    // (text, a token count a real tokenizer gives or exceeds). The low estimate must not exceed it.
    let cases = [
        ("The quick brown fox jumps over the lazy dog", 9),
        ("def add(a, b):\n    return a + b\n", 12),
        (r#"{"a":1,"b":[1,2,3]}"#, 11),
        ("你好，世界", 4),
    ];
    for (text, real) in cases {
        let body = json!({"messages": [{"role": "user", "content": text}]});
        let low = estimate_tokens_low(&body);
        assert!(
            low <= real,
            "{text:?}: low {low} is above a real count of {real}"
        );
        assert!(
            low * 2 <= estimate_tokens(&body) + 4,
            "{text:?}: the low estimate is not clearly below the ordinary one"
        );
    }
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
    // A version-5 file: no incapable_targets column, one legacy decision dated now (the ledger prunes
    // old decisions shortly after opening). Built offline so no live pool holds a stale statement.
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&path);
    let mut conn = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    for statement in [
        "ALTER TABLE decisions DROP COLUMN incapable_targets",
        "INSERT INTO decisions (request_id, ts_ms, format, requested_model, skipped_rules, \
         eligible_targets, stream, tools, images, attempts, status, outcome, latency_ms) \
         VALUES ('before', CAST(strftime('%s', 'now') AS INTEGER) * 1000, 'openai_chat', 'm', \
         '[]', '[]', 0, 0, 0, '[]', 200, 'ok', 5)",
        "PRAGMA user_version = 5",
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
        incapable_targets: vec![humpyard::ledger::Incapable {
            target: "small".into(),
            reason: "tools".into(),
        }],
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
    assert_eq!(
        by_id("before").incapable_targets,
        Vec::<humpyard::ledger::Incapable>::new()
    );
    assert_eq!(by_id("after").incapable_targets[0].reason, "tools");
}

// A per-request cap (`add-endpoint-request-cap`): `groq` declares a huge window and a cap of 100 tokens.

#[tokio::test]
async fn a_prompt_over_a_per_request_cap_skips_the_target_however_large_its_window() {
    let rig = rig().await;
    let (status, target, _) = rig
        .send(&plain_request("cap-ladder", &"word ".repeat(2400)))
        .await;
    assert_eq!((status, target.as_str()), (200, "big"));
    assert_eq!(rig.capped.calls(), 0, "the capped target was never asked");
    let text = rig.metrics().await;
    assert!(
        text.contains(
            r#"humpyard_capability_skips_total{route="cap-ladder",target="groq",reason="request_cap"} 1"#
        ),
        "{text}"
    );
    assert_eq!(
        rig.decision("cap-ladder").await["incapable_targets"],
        json!([{"target": "groq", "reason": "request_cap"}])
    );
}

#[tokio::test]
async fn a_prompt_under_the_cap_is_still_served_by_the_capped_target() {
    let rig = rig().await;
    let (status, target, _) = rig.send(&plain_request("cap-ladder", "hi")).await;
    assert_eq!((status, target.as_str()), (200, "groq"));
    assert_eq!(rig.capped.calls(), 1);
}

#[tokio::test]
async fn a_prompt_at_the_cap_stays_and_one_over_it_goes() {
    let rig = rig().await;
    let at = rig
        .explain(&json!({"model": "cap-ladder", "prompt_tokens": 100}))
        .await;
    assert_eq!(at["incapable_targets"], json!([]));
    let over = rig
        .explain(&json!({"model": "cap-ladder", "prompt_tokens": 101}))
        .await;
    assert_eq!(
        over["incapable_targets"],
        json!([{"target": "groq", "reason": "request_cap"}])
    );
}

#[tokio::test]
async fn a_target_with_a_capped_and_an_uncapped_endpoint_stays() {
    let rig = rig().await;
    let (status, target, _) = rig
        .send(&plain_request("cap-mixed", &"word ".repeat(2400)))
        .await;
    assert_eq!((status, target.as_str()), (200, "mixed-cap"));
    assert_eq!(
        rig.decision("cap-mixed").await["incapable_targets"],
        json!([])
    );
}

#[tokio::test]
async fn a_request_every_target_caps_out_is_refused_naming_the_cap() {
    let rig = rig().await;
    let (status, _, body) = rig
        .send(&plain_request("only-groq", &"word ".repeat(2400)))
        .await;
    assert_eq!(status, 400, "{body}");
    let message = body["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("`groq`") && message.contains("max_request_tokens"),
        "{message}"
    );
    assert_eq!(rig.capped.calls(), 0, "refused before any upstream call");
    let decision = rig.decision("only-groq").await;
    assert_eq!(decision["outcome"], "no_capable_target");
    assert_eq!(
        decision["incapable_targets"],
        json!([{"target": "groq", "reason": "request_cap"}])
    );
}
