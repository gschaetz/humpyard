//! The routing decision log: one row per request (including refusals and cancellations), attempts
//! in order, joined to usage by request id, readable per key, and never able to hurt a request.
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
use humpyard::ledger::{Decision, DecisionQuery, Ledger};
use humpyard::server;
use serde_json::{Value, json};

/// An upstream that answers 200, or the given status, or stalls (status 0).
struct Upstream {
    status: AtomicU16,
}

async fn completions(State(up): State<Arc<Upstream>>) -> axum::response::Response {
    match up.status.load(Ordering::SeqCst) {
        200 => axum::Json(chat_completion("m", "ok", Some(usage(10, 5)))).into_response(),
        0 => {
            tokio::time::sleep(Duration::from_secs(5)).await;
            StatusCode::OK.into_response()
        }
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

/// Targets `a` (first) and `b`; routes `plain` (a), `fallback` (a then b) and `premium` (b);
/// keys `alice`, `bob` (limited to `plain`) and `tight` (1-token budget).
async fn rig(first_status: u16, extra: &str) -> Rig {
    let (first, a_url) = upstream(first_status).await;
    let (_second, b_url) = upstream(200).await;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ledger.db");
    let mut keys = HashMap::new();
    let mut blocks = Vec::new();
    for (id, extra) in [
        ("alice", ""),
        ("bob", "allowed_routes = [\"plain\", \"fallback\"]"),
        ("tight", "daily_tokens = 1"),
    ] {
        let (key, hash) = humpyard::auth::generate_key().unwrap();
        blocks.push(format!("[keys.{id}]\nsha256 = \"{hash}\"\n{extra}"));
        keys.insert(id, key);
    }
    let toml = format!(
        r#"listen = "127.0.0.1:0"
[ledger]
path = "{db}"
{keys}
[providers.pa]
base_url = "{a_url}"
api_key_env = "K"
max_retries = 0
timeout_secs = 3
[providers.pb]
base_url = "{b_url}"
api_key_env = "K"
max_retries = 0
[targets.a]
endpoints = [{{ provider = "pa", model = "ma" }}]
[targets.b]
endpoints = [{{ provider = "pb", model = "mb" }}]
[routes.plain]
type = "passthrough"
targets = ["a"]
[routes.fallback]
type = "passthrough"
targets = ["a", "b"]
[routes.premium]
type = "passthrough"
targets = ["b"]
{extra}
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
    async fn ask(&self, key: &str, model: &str, headers: &[(&str, &str)]) -> reqwest::Response {
        let mut req = reqwest::Client::new()
            .post(format!("{}/v1/chat/completions", self.url))
            .bearer_auth(&self.keys[key])
            .json(&chat_request(model, false));
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        req.send().await.unwrap()
    }

    /// The decisions the API shows `key`, newest first.
    async fn decisions(&self, key: &str, query: &str) -> Vec<Value> {
        let resp = reqwest::Client::new()
            .get(format!("{}/v1/decisions{query}", self.url))
            .bearer_auth(&self.keys[key])
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "{query}");
        let body: Value = resp.json().await.unwrap();
        body["data"].as_array().unwrap().clone()
    }

    /// Every decision in the database, whoever made it (refusals of anonymous callers included).
    async fn all_decisions(&self) -> Vec<Decision> {
        tokio::time::sleep(Duration::from_millis(300)).await;
        Ledger::open(&self.db)
            .await
            .unwrap()
            .decisions(&DecisionQuery {
                limit: 500,
                ..DecisionQuery::default()
            })
            .await
            .unwrap()
    }
}

fn id_of(resp: &reqwest::Response) -> String {
    resp.headers()["x-humpyard-request-id"]
        .to_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn a_served_request_has_an_id_a_decision_and_matching_usage() {
    let r = rig(200, "").await;
    let resp = r.ask("alice", "plain", &[]).await;
    assert_eq!(resp.status(), 200);
    let id = id_of(&resp);
    assert_eq!(id.len(), 16);

    let decisions = r.decisions("alice", "").await;
    assert_eq!(decisions.len(), 1, "{decisions:?}");
    let d = &decisions[0];
    assert_eq!(d["request_id"], id.as_str());
    assert_eq!(d["key"], "alice");
    assert_eq!(
        (d["requested_model"].as_str(), d["route"].as_str()),
        (Some("plain"), Some("plain"))
    );
    assert_eq!(
        (d["rule"].as_str(), d["source"].as_str()),
        (Some("default"), Some("requested_model"))
    );
    assert_eq!(d["outcome"], "ok");
    assert_eq!(d["status"], 200);
    assert_eq!(d["served"]["target"], "a");
    assert_eq!(d["served"]["provider"], "pa");
    assert_eq!(d["eligible_targets"], json!(["a"]));
    assert_eq!(d["budget"], "healthy");
    assert_eq!(d["attempts"].as_array().unwrap().len(), 1);
    assert_eq!(d["attempts"][0]["outcome"], "ok");
    assert_eq!(d["format"], "openai_chat");

    // The usage row carries the same id.
    let rows = Ledger::open(&r.db)
        .await
        .unwrap()
        .entries_since(0)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].request_id.as_deref(), Some(id.as_str()));
}

#[tokio::test]
async fn prompts_and_responses_never_reach_the_database() {
    let r = rig(200, "").await;
    let secret = "UNIQUE-PROMPT-MARKER-7f3a9c";
    let body = json!({"model": "plain", "messages": [{"role": "user", "content": secret}]});
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", r.url))
        .bearer_auth(&r.keys["alice"])
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    // A refusal whose message could tempt an implementation to echo input.
    assert_eq!(r.ask("alice", "no-such-model", &[]).await.status(), 404);
    // The ledger writes off the request path, so wait until both decisions and the usage row are in it (a fixed pause
    // was too short on a slow runner). Looking for the prompt before then would pass for the wrong reason.
    let ledger = Ledger::open(&r.db).await.unwrap();
    let both_decisions = DecisionQuery {
        limit: 10,
        ..DecisionQuery::default()
    };
    let mut written = false;
    for _ in 0..200 {
        let decisions = ledger.decisions(&both_decisions).await.unwrap().len();
        let usage = ledger.entries_since(0).await.unwrap().len();
        if decisions >= 2 && usage >= 1 {
            written = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        written,
        "the ledger never held the two decisions and the usage row"
    );
    ledger.flush().await;
    let mut files = String::new();
    for suffix in ["", "-wal"] {
        if let Ok(bytes) = std::fs::read(format!("{}{suffix}", r.db.display())) {
            files.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    assert!(
        files.contains("alice"),
        "sanity: the files hold the metadata"
    );
    assert!(
        !files.contains("UNIQUE-PROMPT-MARKER"),
        "the prompt text leaked into the database files"
    );
}

#[tokio::test]
async fn a_fallback_shows_every_attempt_in_order() {
    let r = rig(503, "").await;
    let resp = r.ask("alice", "fallback", &[]).await;
    assert_eq!(resp.status(), 200);
    let d = r
        .decisions("alice", &format!("?request_id={}", id_of(&resp)))
        .await;
    assert_eq!(d.len(), 1);
    let attempts = d[0]["attempts"].as_array().unwrap();
    let summary: Vec<(&str, &str, &str)> = attempts
        .iter()
        .map(|a| {
            (
                a["target"].as_str().unwrap(),
                a["provider"].as_str().unwrap(),
                a["outcome"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(summary, [("a", "pa", "server_error"), ("b", "pb", "ok")]);
    assert_eq!(d[0]["served"]["target"], "b");
    assert_eq!(d[0]["outcome"], "ok");
    assert_eq!(r.first.status.load(Ordering::SeqCst), 503);
}

#[tokio::test]
async fn failures_and_refusals_are_decisions_too() {
    let r = rig(503, "").await;
    // Upstream failure with nothing to fall back to.
    let failed = r.ask("alice", "plain", &[]).await;
    assert_eq!(failed.status(), 502);
    assert!(
        failed.headers().contains_key("x-humpyard-request-id"),
        "ids on errors too"
    );
    // Forbidden, unknown model, unauthenticated.
    assert_eq!(r.ask("bob", "premium", &[]).await.status(), 403);
    assert_eq!(r.ask("alice", "nope", &[]).await.status(), 404);
    let anonymous = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", r.url))
        .json(&chat_request("plain", false))
        .send()
        .await
        .unwrap();
    assert_eq!(anonymous.status(), 401);
    // A key past its budget.
    r.first.status.store(200, Ordering::SeqCst);
    assert_eq!(r.ask("tight", "plain", &[]).await.status(), 200);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(r.ask("tight", "plain", &[]).await.status(), 402);

    let all = r.all_decisions().await;
    let by_outcome: HashMap<&str, &Decision> =
        all.iter().map(|d| (d.outcome.as_str(), d)).collect();
    for outcome in [
        "upstream_error",
        "forbidden",
        "unknown_model",
        "unauthorized",
        "budget_exhausted",
    ] {
        assert!(
            by_outcome.contains_key(outcome),
            "missing {outcome}: {:?}",
            all.iter().map(|d| &d.outcome).collect::<Vec<_>>()
        );
    }
    let forbidden = by_outcome["forbidden"];
    assert_eq!(
        (forbidden.status, forbidden.key_id.as_deref()),
        (403, Some("bob"))
    );
    assert!(forbidden.attempts.is_empty() && forbidden.route.is_none());
    assert!(forbidden.message.as_deref().unwrap().contains("premium"));
    assert_eq!(by_outcome["unauthorized"].key_id, None);
    assert_eq!(by_outcome["budget_exhausted"].status, 402);
    let upstream_error = by_outcome["upstream_error"];
    assert_eq!(upstream_error.attempts.len(), 1);
    assert_eq!(upstream_error.attempts[0].outcome, "server_error");
}

#[tokio::test]
async fn rules_skipped_for_the_key_and_the_request_features_are_recorded() {
    let rules = "[[select]]\nname = \"deep\"\nwhen = { profile = \"deep\" }\nroute = \"premium\"\n\
                 [[select]]\nname = \"tools\"\nwhen = { tools = true }\nroute = \"plain\"\n";
    let r = rig(200, rules).await;
    // bob may not use `premium`, so the `deep` rule is skipped and the request follows the model.
    let resp = r
        .ask("bob", "plain", &[("x-humpyard-profile", "deep")])
        .await;
    assert_eq!(resp.status(), 200);
    let d = &r.decisions("bob", "").await[0];
    assert_eq!(d["route"], "plain");
    assert_eq!(d["rule"], "default");
    assert_eq!(d["skipped_rules"], json!(["deep"]));

    // alice gets the rule, and the tools flag shows in the recorded features.
    let body = json!({"model": "plain", "messages": [{"role": "user", "content": "hi"}],
        "tools": [{"type": "function", "function": {"name": "ls", "parameters": {"type": "object", "properties": {}}}}]});
    let resp = reqwest::Client::new()
        .post(format!("{}/v1/chat/completions", r.url))
        .bearer_auth(&r.keys["alice"])
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let d = &r.decisions("alice", "").await[0];
    assert_eq!(
        (d["rule"].as_str(), d["source"].as_str()),
        (Some("tools"), Some("selector"))
    );
    assert_eq!(d["request"]["tools"], true);
    assert_eq!(d["request"]["stream"], false);
}

#[tokio::test]
async fn a_key_reads_only_its_own_decisions_and_filters_work() {
    let r = rig(200, "").await;
    for _ in 0..3 {
        r.ask("alice", "plain", &[]).await;
    }
    r.ask("alice", "fallback", &[]).await;
    r.ask("bob", "plain", &[]).await;
    assert_eq!(r.decisions("alice", "").await.len(), 4);
    assert_eq!(r.decisions("bob", "").await.len(), 1);
    assert_eq!(r.decisions("alice", "?route=fallback").await.len(), 1);
    assert_eq!(r.decisions("alice", "?limit=2").await.len(), 2);
    assert_eq!(r.decisions("alice", "?outcome=forbidden").await.len(), 0);
    assert_eq!(
        r.decisions("alice", "?since_ms=99999999999999").await.len(),
        0
    );
    // Newest first.
    let all = r.decisions("alice", "").await;
    let times: Vec<i64> = all.iter().map(|d| d["ts_ms"].as_i64().unwrap()).collect();
    assert!(times.windows(2).all(|w| w[0] >= w[1]), "{times:?}");
    // Another key's request id yields nothing.
    let bob_id = r.decisions("bob", "").await[0]["request_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        r.decisions("alice", &format!("?request_id={bob_id}"))
            .await
            .len(),
        0
    );

    let bad = reqwest::Client::new()
        .get(format!("{}/v1/decisions?bogus=1", r.url))
        .bearer_auth(&r.keys["alice"])
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
    let anonymous = reqwest::get(format!("{}/v1/decisions", r.url))
        .await
        .unwrap();
    assert_eq!(anonymous.status(), 401);
}

#[tokio::test]
async fn an_abandoned_request_is_recorded_as_cancelled() {
    let r = rig(0, "").await; // the upstream never answers within the client's patience
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(400))
        .build()
        .unwrap();
    let gone = client
        .post(format!("{}/v1/chat/completions", r.url))
        .bearer_auth(&r.keys["alice"])
        .json(&chat_request("plain", false))
        .send()
        .await;
    assert!(gone.is_err(), "the client gave up");
    tokio::time::sleep(Duration::from_millis(600)).await;
    let d = r.decisions("alice", "").await;
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0]["outcome"], "cancelled");
    assert_eq!(d[0]["route"], "plain");
}

#[tokio::test]
async fn a_broken_decision_table_never_affects_requests() {
    let r = rig(200, "").await;
    assert_eq!(r.ask("alice", "plain", &[]).await.status(), 200);
    tokio::time::sleep(Duration::from_millis(300)).await;
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{}", r.db.display()))
        .await
        .unwrap();
    sqlx::query("DROP TABLE decisions")
        .execute(&pool)
        .await
        .unwrap();
    for _ in 0..3 {
        assert_eq!(
            r.ask("alice", "plain", &[]).await.status(),
            200,
            "recording failures are not the client's problem"
        );
    }
}

#[tokio::test]
async fn without_a_ledger_there_is_nothing_to_read() {
    let (_up, url) = upstream(200).await;
    let toml = format!(
        "listen = \"127.0.0.1:0\"\n[providers.p]\nbase_url = \"{url}\"\napi_key_env = \"K\"\n[targets.t]\nendpoints = [{{ provider = \"p\", model = \"m\" }}]\n"
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    let base = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    let resp = reqwest::get(format!("{base}/v1/decisions")).await.unwrap();
    assert_eq!(resp.status(), 404);
    // Requests still get ids.
    let ok = reqwest::Client::new()
        .post(format!("{base}/v1/chat/completions"))
        .json(&chat_request("t", false))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
    assert!(ok.headers().contains_key("x-humpyard-request-id"));
}

#[tokio::test]
async fn old_decisions_are_pruned_and_a_v1_database_is_upgraded_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    // A database as version 1 (before decisions) left it, with one usage row.
    {
        let pool = sqlx::SqlitePool::connect_with(
            sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .unwrap();
        sqlx::query("CREATE TABLE usage (id INTEGER PRIMARY KEY AUTOINCREMENT, ts_ms INTEGER NOT NULL, key_id TEXT, session_id TEXT, route TEXT NOT NULL, target TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL, kind TEXT NOT NULL, input_tokens INTEGER NOT NULL, cached_input_tokens INTEGER NOT NULL, cache_creation_tokens INTEGER NOT NULL, output_tokens INTEGER NOT NULL, reasoning_tokens INTEGER NOT NULL, cost_micro_usd INTEGER NOT NULL, outcome TEXT NOT NULL, usage_missing INTEGER NOT NULL)")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO usage (ts_ms, key_id, route, target, provider, model, kind, input_tokens, cached_input_tokens, cache_creation_tokens, output_tokens, reasoning_tokens, cost_micro_usd, outcome, usage_missing) VALUES (1, 'k', 'r', 't', 'p', 'm', 'answer', 5, 0, 0, 5, 0, 7, 'ok', 0)")
            .execute(&pool).await.unwrap();
        sqlx::query("PRAGMA user_version = 1")
            .execute(&pool)
            .await
            .unwrap();
    }
    let ledger = Ledger::open_with(&path, 100, 1).await.unwrap(); // keep decisions for 1 day
    let rows = ledger.entries_since(0).await.unwrap();
    assert_eq!(rows.len(), 1, "old usage survives the upgrade");
    assert_eq!(
        (rows[0].cost_micro_usd, rows[0].request_id.clone()),
        (7, None)
    );

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let now = i64::try_from(now).unwrap();
    let decision = |id: &str, ts_ms: i64| Decision {
        request_id: id.into(),
        ts_ms,
        outcome: "ok".into(),
        status: 200,
        ..Decision::default()
    };
    ledger.record_decision(decision("ancient", now - 3 * 86_400_000));
    ledger.record_decision(decision("recent", now));
    ledger.flush().await;
    tokio::time::sleep(Duration::from_millis(400)).await; // the writer prunes after its batch
    let kept = ledger
        .decisions(&DecisionQuery {
            limit: 10,
            ..DecisionQuery::default()
        })
        .await
        .unwrap();
    let ids: Vec<&str> = kept.iter().map(|d| d.request_id.as_str()).collect();
    assert_eq!(
        ids,
        ["recent"],
        "the 3-day-old decision is past its 1-day retention"
    );
}

#[tokio::test]
async fn long_error_text_is_clipped() {
    let long = "x".repeat(900);
    assert_eq!(Decision::clip(&long).chars().count(), 200);
    assert_eq!(Decision::clip("short"), "short");
}

#[tokio::test]
async fn a_client_supplied_model_name_is_clipped_before_it_is_stored() {
    let r = rig(200, "").await;
    let huge = "m".repeat(5000);
    assert_eq!(r.ask("alice", &huge, &[]).await.status(), 404);
    let d = &r.decisions("alice", "").await[0];
    assert_eq!(d["requested_model"].as_str().unwrap().chars().count(), 200);
    assert!(d["message"].as_str().unwrap().chars().count() <= 200);
}
