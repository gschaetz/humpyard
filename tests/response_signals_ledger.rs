//! Ledger schema v13: four nullable columns on `usage` hold the facts about an answer (spec: `usage-ledger`,
//! change `record-response-quality-signals`, ADR 0030). A row from before them, a judge call and an unobserved
//! call read as unknown; an observed call with no fact set reads as known and clean.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

use humpyard::ledger::{Entry, Kind, Ledger};
use humpyard::signals::Signals;
use sqlx::Connection;

fn entry(request_id: &str, signals: Option<Signals>) -> Entry {
    Entry {
        ts_ms: 1_000,
        key_id: Some("k".into()),
        session_id: None,
        route: "auto".into(),
        target: "fast".into(),
        provider: "mock".into(),
        model: "m".into(),
        kind: Kind::Answer,
        input_tokens: 1,
        cached_input_tokens: 0,
        cache_creation_tokens: 0,
        output_tokens: 1,
        reasoning_tokens: 0,
        cost_micro_usd: 1,
        outcome: "ok".into(),
        usage_missing: false,
        request_id: Some(request_id.into()),
        signals,
    }
}

fn options(path: &std::path::Path) -> sqlx::sqlite::SqliteConnectOptions {
    sqlx::sqlite::SqliteConnectOptions::new().filename(path)
}

#[tokio::test]
async fn every_combination_round_trips_and_unknown_stays_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let ledger = Ledger::open(&dir.path().join("l.db")).await.unwrap();
    let mut wanted = vec![("unknown".to_string(), None)];
    for bits in 0..16u8 {
        wanted.push((
            format!("bits-{bits}"),
            Some(Signals {
                empty_output: bits & 1 != 0,
                truncated_output: bits & 2 != 0,
                bad_tool_call: bits & 4 != 0,
                cut_stream: bits & 8 != 0,
            }),
        ));
    }
    for (id, signals) in &wanted {
        ledger.record(entry(id, *signals));
    }
    ledger.flush().await;
    let rows = ledger.entries_since(0).await.unwrap();
    assert_eq!(rows.len(), wanted.len());
    for ((id, signals), row) in wanted.iter().zip(&rows) {
        assert_eq!(row.request_id.as_deref(), Some(id.as_str()));
        assert_eq!(row.signals, *signals, "{id}");
    }
    ledger.shutdown().await;
}

#[tokio::test]
async fn a_v12_file_is_upgraded_reads_old_rows_as_unknown_and_takes_new_ones() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.db");
    let ledger = Ledger::open(&path).await.unwrap();
    ledger.record(entry("old", None));
    ledger.flush().await;
    ledger.shutdown().await;
    // A version-12 file: no signal columns. Built offline so no live pool holds a stale statement.
    let mut conn = sqlx::SqliteConnection::connect_with(&options(&path))
        .await
        .unwrap();
    for statement in [
        "ALTER TABLE usage DROP COLUMN empty_output",
        "ALTER TABLE usage DROP COLUMN truncated_output",
        "ALTER TABLE usage DROP COLUMN bad_tool_call",
        "ALTER TABLE usage DROP COLUMN cut_stream",
        "PRAGMA user_version = 12",
    ] {
        sqlx::query(statement).execute(&mut conn).await.unwrap();
    }
    conn.close().await.unwrap();

    let flagged = Signals {
        truncated_output: true,
        ..Signals::default()
    };
    let ledger = Ledger::open(&path).await.unwrap();
    ledger.record(entry("new", Some(flagged)));
    ledger.flush().await;
    let rows = ledger.entries_since(0).await.unwrap();
    assert_eq!(rows.len(), 2, "the old row survives the upgrade");
    assert_eq!(rows[0].signals, None, "an old row reads as unknown");
    assert_eq!(rows[1].signals, Some(flagged));
    ledger.shutdown().await;

    // Reopening a migrated file is harmless and changes nothing.
    let ledger = Ledger::open(&path).await.unwrap();
    let rows = ledger.entries_since(0).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].signals, None);
    assert_eq!(rows[1].signals, Some(flagged));
    ledger.shutdown().await;

    let mut conn = sqlx::SqliteConnection::connect_with(&options(&path).read_only(true))
        .await
        .unwrap();
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&mut conn)
        .await
        .unwrap();
    assert_eq!(version, 13);
}
