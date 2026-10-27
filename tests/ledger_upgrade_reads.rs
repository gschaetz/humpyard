//! Reading decisions right after an upgrade that added a column. A pooled connection that loaded the schema before
//! the migration's `ALTER TABLE` kept the old column list, and sqlx then panicked in its worker thread on the next
//! `SELECT *` (its cached columns were fewer than the re-prepared statement's), so the query silently returned
//! nothing. About one in three hundred opens failed; the fix migrates before any pooled connection exists.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // test scaffolding

use humpyard::ledger::{Decision, DecisionQuery, Ledger};
use sqlx::Connection;

fn now_ms() -> i64 {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    i64::try_from(since).unwrap()
}

/// A ledger file as the previous schema version left it: one decision, and no `repaired_tool_arguments` column.
async fn previous_version(path: &std::path::Path) {
    let ledger = Ledger::open(path).await.unwrap();
    ledger.record_decision(Decision {
        request_id: "old".into(),
        ts_ms: now_ms(),
        outcome: "ok".into(),
        status: 200,
        ..Decision::default()
    });
    ledger.flush().await;
    ledger.shutdown().await;
    let options = sqlx::sqlite::SqliteConnectOptions::new().filename(path);
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
}

#[tokio::test]
async fn every_pooled_connection_reads_decisions_after_a_column_was_added() {
    for round in 0..5 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        previous_version(&path).await;

        let ledger = Ledger::open(&path).await.unwrap();
        let query = DecisionQuery {
            limit: 10,
            ..DecisionQuery::default()
        };
        let read = || ledger.decisions(&query);
        // Concurrent reads use every connection in the pool, including one opened before the migration ran.
        let (a, b, c, d) = tokio::join!(read(), read(), read(), read());
        for rows in [a, b, c, d] {
            let rows = rows.unwrap();
            assert_eq!(rows.len(), 1, "round {round}: a read returned {rows:?}");
            assert_eq!(rows[0].request_id, "old");
        }
        ledger.shutdown().await;
    }
}
