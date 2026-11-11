//! The ledger's read pool (ADR 0036): reads use connections of their own that refuse writes and give up quickly, so
//! a read never takes a connection the writer needs and never holds a request for the library's default waits.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration tests fail loudly on purpose

mod common;

use std::time::{Duration, Instant};

use common::seed::{Rng, make_row};
use humpyard::ledger::{DecisionQuery, Ledger, LedgerError, READ_ACQUIRE_TIMEOUT};

async fn ledger(name: &str) -> Ledger {
    let dir = common::scratch().join(format!("read-pool-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Ledger::open(&dir.join("ledger.db")).await.unwrap()
}

async fn count(ledger: &Ledger) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM usage")
        .fetch_one(ledger.pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn a_write_through_the_read_pool_is_refused() {
    let ledger = ledger("refuses").await;
    let error = sqlx::query("CREATE TABLE scratch (a INTEGER)")
        .execute(ledger.read_pool())
        .await
        .unwrap_err();
    assert!(
        error.to_string().to_lowercase().contains("readonly"),
        "{error}"
    );
    let error = sqlx::query("DELETE FROM usage")
        .execute(ledger.read_pool())
        .await
        .unwrap_err();
    assert!(
        error.to_string().to_lowercase().contains("readonly"),
        "{error}"
    );
    // The writer's pool still writes.
    sqlx::query("CREATE TABLE scratch (a INTEGER)")
        .execute(ledger.pool())
        .await
        .unwrap();
}

#[tokio::test]
async fn holding_both_read_connections_does_not_delay_a_write() {
    let ledger = ledger("holding").await;
    let _first = ledger.read_pool().acquire().await.unwrap();
    let _second = ledger.read_pool().acquire().await.unwrap();

    let (entry, _) = make_row(&mut Rng(1), 0, 1_000);
    ledger.record(entry);
    tokio::time::timeout(Duration::from_secs(1), ledger.flush())
        .await
        .expect("the write waited for a read connection");
    assert_eq!(count(&ledger).await, 1);
}

#[tokio::test]
async fn a_third_read_fails_as_busy_within_about_two_seconds() {
    let ledger = ledger("third").await;
    let _first = ledger.read_pool().acquire().await.unwrap();
    let _second = ledger.read_pool().acquire().await.unwrap();

    let started = Instant::now();
    let error = ledger
        .decisions(&DecisionQuery::default())
        .await
        .unwrap_err();
    let waited = started.elapsed();
    assert!(
        error.is_busy(),
        "a pool timeout is busy, not broken: {error}"
    );
    assert!(
        waited + Duration::from_millis(200) >= READ_ACQUIRE_TIMEOUT
            && waited < READ_ACQUIRE_TIMEOUT + Duration::from_secs(1),
        "gave up after {waited:?}"
    );
    // And the other reads give up the same way.
    assert!(ledger.entries_since(0).await.unwrap_err().is_busy());
}

#[tokio::test]
async fn nothing_is_dropped_while_reads_are_held() {
    let ledger = ledger("dropped").await;
    let _first = ledger.read_pool().acquire().await.unwrap();
    let _second = ledger.read_pool().acquire().await.unwrap();

    let mut rng = Rng(7);
    for i in 0..2_000 {
        let (entry, decision) = make_row(&mut rng, i, 1_000 + i64::try_from(i).unwrap());
        ledger.record(entry);
        ledger.record_decision(decision);
    }
    tokio::time::timeout(Duration::from_secs(10), ledger.flush())
        .await
        .expect("the writer stalled behind held reads");
    assert_eq!(ledger.dropped(), 0);
    assert_eq!(ledger.failed(), 0);
    assert_eq!(count(&ledger).await, 2_000);
}

#[tokio::test]
async fn a_read_sees_what_the_writer_has_flushed() {
    let ledger = ledger("sees").await;
    // A current timestamp: the writer prunes decisions older than the retention after its first batch.
    let now_ms = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    let (entry, decision) = make_row(&mut Rng(3), 0, now_ms);
    ledger.record(entry);
    ledger.record_decision(decision);
    ledger.flush().await;
    assert_eq!(ledger.entries_since(0).await.unwrap().len(), 1);
    assert_eq!(
        ledger
            .decisions(&DecisionQuery {
                limit: 100,
                ..DecisionQuery::default()
            })
            .await
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn only_waiting_errors_are_busy() {
    assert!(LedgerError::Query(sqlx::Error::PoolTimedOut).is_busy());
    assert!(!LedgerError::Query(sqlx::Error::RowNotFound).is_busy());
    assert!(!LedgerError::Query(sqlx::Error::PoolClosed).is_busy());
}
