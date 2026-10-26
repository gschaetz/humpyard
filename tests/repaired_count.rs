//! A decision counts the tool calls whose arguments were repaired before routing, and keeps nothing of the
//! arguments (spec: `decision-log`, change `repair-tool-arguments`, ADR 0029). Also the ledger's v12 step that
//! adds the column.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

mod common;

use axum::Router;
use axum::routing::post;
use common::{chat_completion, serve, usage};
use humpyard::config::Config;
use humpyard::ledger::{Decision, DecisionQuery, Ledger};
use humpyard::server;
use serde_json::{Value, json};

const MARKER: &str = "zebra-quartz-marker-arguments";

/// A gateway with one passthrough route `r` over an upstream that answers 200, an admin credential that may read,
/// and a ledger. Returns its URL, the temp dir that holds the ledger and the admin secret.
async fn gateway() -> (String, tempfile::TempDir, String) {
    let app = Router::new().route(
        "/chat/completions",
        post(|| async { axum::Json(chat_completion("m", "ok", Some(usage(10, 5)))) }),
    );
    let upstream = format!("http://{}", serve(app).await);
    let dir = tempfile::tempdir().unwrap();
    let (secret, hash) = humpyard::auth::generate_key().unwrap();
    let toml = format!(
        "listen = \"127.0.0.1:0\"\n[ledger]\npath = \"{}\"\n\
         [admin.credentials.gary]\nsha256 = \"{hash}\"\nscopes = [\"read\"]\n\
         [providers.p]\nbase_url = \"{upstream}\"\napi_key_env = \"K\"\nmax_retries = 0\ntimeout_secs = 5\n\
         [targets.t]\nendpoints = [{{ provider = \"p\", model = \"m\" }}]\n\
         [routes.r]\ntype = \"passthrough\"\ntargets = [\"t\"]\n",
        dir.path().join("ledger.db").display()
    );
    let config = Config::from_toml(&toml, |_| Some("k".into())).unwrap();
    let url = format!(
        "http://{}",
        serve(server::router(config).await.unwrap()).await
    );
    (url, dir, secret)
}

/// Now, in milliseconds: decisions older than the retention are pruned, so test rows must be recent.
fn now_ms() -> i64 {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    i64::try_from(since).unwrap()
}

fn call(id: &str, arguments: &str) -> Value {
    json!({"id": id, "type": "function", "function": {"name": "shell", "arguments": arguments}})
}

/// A chat history whose assistant turn makes the given tool calls.
fn history(model: &str, calls: &[Value]) -> Value {
    json!({
        "model": model,
        "messages": [
            {"role": "user", "content": "go"},
            {"role": "assistant", "content": null, "tool_calls": calls},
            {"role": "user", "content": "go on"}
        ]
    })
}

async fn send(url: &str, body: &Value) -> u16 {
    reqwest::Client::new()
        .post(format!("{url}/v1/chat/completions"))
        .json(body)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

/// The decisions from `/v1/decisions` (newest first) as text and parsed, then from `/admin/v1/decisions`.
async fn decisions(url: &str, admin: &str) -> (Vec<Value>, Vec<Value>, String) {
    let public = reqwest::get(format!("{url}/v1/decisions?limit=10"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let admin_text = reqwest::Client::new()
        .get(format!("{url}/admin/v1/decisions?limit=10"))
        .bearer_auth(admin)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let rows = |text: &str| -> Vec<Value> {
        serde_json::from_str::<Value>(text).unwrap()["data"]
            .as_array()
            .unwrap()
            .clone()
    };
    (
        rows(&public),
        rows(&admin_text),
        format!("{public}{admin_text}"),
    )
}

#[tokio::test]
async fn a_repaired_request_shows_its_count_on_both_endpoints_and_a_clean_one_shows_none() {
    let (url, _dir, admin) = gateway().await;
    let cut = call("a", r#"{"cmd":"ls","workdir":"/tm"#);
    let also_cut = call("b", r#"{"cmd":["#);
    let fine = call("c", r#"{"cmd":"pwd"}"#);
    assert_eq!(
        send(&url, &history("r", &[cut, fine.clone(), also_cut])).await,
        200
    );
    assert_eq!(send(&url, &history("r", &[fine])).await, 200);

    let (public, admin_rows, _) = decisions(&url, &admin).await;
    for rows in [public, admin_rows] {
        assert_eq!(rows.len(), 2);
        // Newest first: the clean request, then the one with two repairs.
        assert!(
            rows[0]["request"].get("repaired_tool_arguments").is_none(),
            "{}",
            rows[0]
        );
        assert_eq!(
            rows[1]["request"]["repaired_tool_arguments"], 2,
            "{}",
            rows[1]
        );
        assert_eq!(rows[1]["request"]["tools"], false);
    }
}

#[tokio::test]
async fn a_request_refused_before_any_provider_is_tried_still_carries_the_count() {
    let (url, _dir, admin) = gateway().await;
    let status = send(&url, &history("no-such-route", &[call("a", r#"{"cmd":"#)])).await;
    assert_eq!(status, 404);

    let (public, admin_rows, _) = decisions(&url, &admin).await;
    for rows in [public, admin_rows] {
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["outcome"], "unknown_model", "{}", rows[0]);
        assert_eq!(
            rows[0]["request"]["repaired_tool_arguments"], 1,
            "{}",
            rows[0]
        );
        assert_eq!(rows[0]["attempts"].as_array().unwrap().len(), 0);
    }
}

#[tokio::test]
async fn no_part_of_the_arguments_reaches_the_decision() {
    let (url, _dir, admin) = gateway().await;
    // The recognizable text sits in the part that is kept and in the part that is cut off.
    let cut = call(
        "a",
        &format!(r#"{{"note":"{MARKER}","more":"{MARKER}-and-th"#),
    );
    let unfixable = call("b", &format!("not json {MARKER}"));
    assert_eq!(send(&url, &history("r", &[cut, unfixable])).await, 200);

    let (public, _, everything) = decisions(&url, &admin).await;
    assert_eq!(public[0]["request"]["repaired_tool_arguments"], 2);
    assert!(!everything.contains(MARKER), "{everything}");
    assert!(!everything.contains("zebra"), "{everything}");
}

#[tokio::test]
async fn a_count_round_trips_through_the_ledger_and_an_unrepaired_decision_has_none() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(&dir.path().join("l.db")).await.unwrap();
    let decision = |id: &str, count: Option<u32>| Decision {
        request_id: id.into(),
        ts_ms: now_ms(),
        outcome: "ok".into(),
        status: 200,
        repaired_tool_arguments: count,
        ..Decision::default()
    };
    ledger.record_decision(decision("with", Some(3)));
    ledger.record_decision(decision("without", None));
    ledger.flush().await;
    let rows = ledger
        .decisions(&DecisionQuery {
            limit: 10,
            ..DecisionQuery::default()
        })
        .await
        .unwrap();
    let count = |id: &str| {
        rows.iter()
            .find(|d| d.request_id == id)
            .unwrap()
            .repaired_tool_arguments
    };
    assert_eq!(count("with"), Some(3));
    assert_eq!(count("without"), None);
    ledger.shutdown().await;
}

#[tokio::test]
async fn an_older_ledger_gains_the_column_and_keeps_its_decisions() {
    use sqlx::Connection;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    let ledger = Ledger::open(&path).await.unwrap();
    ledger.record_decision(Decision {
        request_id: "old".into(),
        ts_ms: now_ms(),
        outcome: "ok".into(),
        status: 200,
        ..Decision::default()
    });
    ledger.flush().await;
    ledger.shutdown().await;
    // A version-11 file: no `repaired_tool_arguments` column. Built offline so no live pool holds a stale statement.
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(&path);
    let mut conn = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    for statement in [
        "ALTER TABLE decisions DROP COLUMN repaired_tool_arguments",
        "PRAGMA user_version = 11",
    ] {
        sqlx::query(statement).execute(&mut conn).await.unwrap();
    }
    conn.close().await.unwrap();

    let ledger = Ledger::open(&path).await.unwrap();
    let rows = ledger
        .decisions(&DecisionQuery {
            limit: 10,
            ..DecisionQuery::default()
        })
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "the old decision survives the upgrade");
    assert_eq!(rows[0].repaired_tool_arguments, None);
    // New rows can carry the count after the upgrade.
    ledger.record_decision(Decision {
        request_id: "new".into(),
        ts_ms: now_ms(),
        outcome: "ok".into(),
        status: 200,
        repaired_tool_arguments: Some(1),
        ..Decision::default()
    });
    ledger.flush().await;
    ledger.shutdown().await;

    let mut conn = sqlx::SqliteConnection::connect_with(&options.read_only(true))
        .await
        .unwrap();
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(version, 12);
    let stored: Option<i64> = sqlx::query_scalar(
        "SELECT repaired_tool_arguments FROM decisions WHERE request_id = 'new'",
    )
    .fetch_one(&mut conn)
    .await
    .unwrap();
    assert_eq!(stored, Some(1));
}
