//! Pruning in batches (ADR 0036): the ledger deletes what is past its retention in short statements, so a large
//! backlog never holds the database's write lock for as long as it takes to delete it, and a prune cut short
//! leaves the rest for the next one.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // integration tests fail loudly on purpose

mod common;

use std::time::{Duration, Instant};

use common::seed::{Rng, make_row};
use humpyard::ledger::{Ledger, NewOverride, Pruned, QuotaSample};

const DAY_MS: i64 = 86_400_000;
const RETENTION_DAYS: u64 = 7;

/// Statements a prune takes beyond the decisions' own when nothing else is old: override history (two tables) and
/// quota samples each find nothing in one statement.
const OTHER_TABLES: u64 = 3;

/// The oldest a row may be and be kept, as the ledger computes it.
fn cutoff_ms() -> i64 {
    now_ms() - i64::try_from(RETENTION_DAYS).unwrap() * DAY_MS
}
/// Rest between batches in these tests: long enough for a waiting writer to take the lock, short enough to be quick.
const PAUSE: Duration = Duration::from_millis(2);

fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

/// A ledger that never prunes by itself (retention 0), so a test prunes when it chooses.
async fn ledger(name: &str) -> Ledger {
    let dir = common::scratch().join(format!("prune-{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Ledger::open_with(&dir.join("ledger.db"), 100_000, 0)
        .await
        .unwrap()
}

/// `old` decisions ten days old and `recent` an hour old.
async fn seed_decisions(ledger: &Ledger, old: u64, recent: u64) {
    let now = now_ms();
    let mut rng = Rng(5);
    for i in 0..old {
        ledger.record_decision(
            make_row(&mut rng, i, now - 10 * DAY_MS - i64::try_from(i).unwrap()).1,
        );
        if ledger.queue_depth() > 50_000 {
            ledger.flush().await;
        }
    }
    for i in 0..recent {
        ledger.record_decision(make_row(&mut rng, old + i, now - 3_600_000).1);
    }
    ledger.flush().await;
    assert_eq!(ledger.dropped(), 0);
}

async fn decisions_before(ledger: &Ledger, cutoff_ms: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM decisions WHERE ts_ms < ?1")
        .bind(cutoff_ms)
        .fetch_one(ledger.pool())
        .await
        .unwrap()
}

async fn decisions_total(ledger: &Ledger) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM decisions")
        .fetch_one(ledger.pool())
        .await
        .unwrap()
}

#[tokio::test]
async fn a_large_backlog_is_deleted_exactly_across_many_batches() {
    let ledger = ledger("exact").await;
    seed_decisions(&ledger, 25_000, 100).await;
    let cutoff = cutoff_ms();
    assert_eq!(decisions_before(&ledger, cutoff).await, 25_000);

    // 13 statements of at most 2,000: twelve full ones and a short one of 1,000, which ends the loop.
    let pruned = ledger.prune_expired(RETENTION_DAYS, 2_000, PAUSE).await;
    assert_eq!(
        pruned,
        Pruned {
            rows: 25_000,
            statements: 13 + OTHER_TABLES
        }
    );

    assert_eq!(
        decisions_before(&ledger, cutoff).await,
        0,
        "every expired row is gone"
    );
    assert_eq!(
        decisions_total(&ledger).await,
        100,
        "and nothing else was touched"
    );
}

#[tokio::test]
async fn nothing_newer_than_the_cutoff_is_touched_and_a_backlog_that_fits_one_batch_is_deleted() {
    let ledger = ledger("newer").await;
    seed_decisions(&ledger, 50, 40).await;
    let before: Vec<String> =
        sqlx::query_scalar("SELECT request_id FROM decisions WHERE ts_ms >= ?1 ORDER BY id")
            .bind(now_ms() - DAY_MS)
            .fetch_all(ledger.pool())
            .await
            .unwrap();
    let pruned = ledger.prune_expired(RETENTION_DAYS, 10_000, PAUSE).await;
    assert_eq!(
        pruned,
        Pruned {
            rows: 50,
            statements: 1 + OTHER_TABLES
        },
        "a backlog that fits one batch is one statement"
    );
    let after: Vec<String> = sqlx::query_scalar("SELECT request_id FROM decisions ORDER BY id")
        .fetch_all(ledger.pool())
        .await
        .unwrap();
    assert_eq!(after, before, "the same recent rows, in the same order");
    assert_eq!(after.len(), 40);

    // A batch that is exactly full ends with one more statement that deletes nothing.
    let ledger = self::ledger("exactly-full").await;
    seed_decisions(&ledger, 300, 5).await;
    let pruned = ledger.prune_expired(RETENTION_DAYS, 100, PAUSE).await;
    assert_eq!(
        pruned,
        Pruned {
            rows: 300,
            statements: 4 + OTHER_TABLES
        },
        "three full statements, then one that finds nothing left and ends the loop"
    );
    assert_eq!(decisions_total(&ledger).await, 5);
}

#[tokio::test]
async fn quota_samples_and_override_history_are_pruned_the_same_way() {
    let ledger = ledger("others").await;
    let now = now_ms();
    let sample = |ts_ms: i64, n: u32| QuotaSample {
        ts_ms,
        provider: "p".to_string(),
        window: format!("w{n}"),
        percent: 1.0,
        status: "ok".to_string(),
        resets_at: None,
    };
    ledger.record_quota((0..3_500).map(|n| sample(now - 20 * DAY_MS, n)).collect());
    ledger.record_quota(vec![sample(now - 60_000, 9_999)]);
    ledger.flush().await;

    let db = ledger.override_db();
    let over = |created_ms: i64, expires_ms: i64| NewOverride {
        kind: "disable_target".to_string(),
        subject: "t".to_string(),
        route: None,
        reason: "test".to_string(),
        created_ms,
        expires_ms,
        created_by: "k".to_string(),
    };
    db.create(&over(now - 20 * DAY_MS, now - 19 * DAY_MS))
        .await
        .unwrap(); // ended long ago
    let recent = db
        .create(&over(now - 60_000, now + 3_600_000))
        .await
        .unwrap(); // in force

    ledger.prune_expired(RETENTION_DAYS, 1_000, PAUSE).await;

    let samples: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM quota_samples")
        .fetch_one(ledger.pool())
        .await
        .unwrap();
    assert_eq!(samples, 1, "the 3,500 old samples went in four batches");
    let kept: Vec<i64> = sqlx::query_scalar("SELECT id FROM overrides ORDER BY id")
        .fetch_all(ledger.pool())
        .await
        .unwrap();
    assert_eq!(
        kept,
        [recent],
        "the override still in force is never pruned"
    );
    let audit: Vec<i64> = sqlx::query_scalar("SELECT override_id FROM override_audit")
        .fetch_all(ledger.pool())
        .await
        .unwrap();
    assert_eq!(audit, [recent], "and so is its history");
}

#[tokio::test]
async fn a_prune_cut_short_leaves_the_rest_for_the_next_one() {
    let ledger = ledger("interrupted").await;
    seed_decisions(&ledger, 20_000, 60).await;
    let cutoff = cutoff_ms();

    let pruning = {
        let ledger = std::sync::Arc::new(ledger);
        let task = {
            let ledger = ledger.clone();
            tokio::spawn(async move {
                ledger
                    .prune_expired(RETENTION_DAYS, 1_000, Duration::from_millis(60))
                    .await;
            })
        };
        // Cut it off once a batch or two has gone, during the rest between batches.
        let started = Instant::now();
        while decisions_before(&ledger, cutoff).await >= 20_000 {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "the prune never started"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        task.abort();
        let _ = task.await;
        ledger
    };

    let left = decisions_before(&pruning, cutoff).await;
    assert!(left > 0 && left < 20_000, "{left} expired rows left");
    assert_eq!(
        decisions_total(&pruning).await,
        left + 60,
        "no recent row was lost"
    );
    assert_eq!(
        left % 1_000,
        0,
        "each statement either deleted its whole batch or did not run"
    );

    pruning.prune_expired(RETENTION_DAYS, 1_000, PAUSE).await;
    assert_eq!(
        decisions_before(&pruning, cutoff).await,
        0,
        "the next prune finishes the job"
    );
    assert_eq!(decisions_total(&pruning).await, 60);
}

fn an_override(n: i64) -> NewOverride {
    NewOverride {
        kind: "disable_target".to_string(),
        subject: format!("t{n}"),
        route: None,
        reason: "isolation".to_string(),
        created_ms: now_ms(),
        expires_ms: now_ms() + 3_600_000,
        created_by: "k".to_string(),
    }
}

/// What override writes saw while a prune ran: the slowest wait, and how many writes completed while the prune was
/// still working (a prune that holds the lock throughout lets none through until it ends).
struct Writes {
    slowest: Duration,
    during_prune: usize,
}

/// Writes overrides back to back from a task of its own while `prune` runs.
async fn override_writes_during<F>(ledger: std::sync::Arc<Ledger>, prune: F) -> Writes
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writer = {
        let (ledger, stop) = (ledger.clone(), stop.clone());
        tokio::spawn(async move {
            let (mut slowest, mut done, mut n) = (Duration::ZERO, Vec::new(), 0);
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let started = Instant::now();
                ledger.override_db().create(&an_override(n)).await.unwrap();
                slowest = slowest.max(started.elapsed());
                done.push(Instant::now());
                n += 1;
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            (slowest, done)
        })
    };
    tokio::time::sleep(Duration::from_millis(30)).await; // writes are flowing before the prune starts
    let (began, ended) = (Instant::now(), {
        prune.await;
        Instant::now()
    });
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let (slowest, done) = writer.await.unwrap();
    Writes {
        slowest,
        during_prune: done.iter().filter(|t| **t > began && **t < ended).count(),
    }
}

#[tokio::test]
async fn writes_make_progress_while_a_large_backlog_is_pruned() {
    const BACKLOG: u64 = 60_000;
    // The old behaviour, as a control: the whole backlog in one statement holds the lock until it is done.
    let single = std::sync::Arc::new(ledger("single").await);
    seed_decisions(&single, BACKLOG, 10).await;
    let cutoff = cutoff_ms();
    let one_statement = override_writes_during(single.clone(), {
        let ledger = single.clone();
        async move {
            sqlx::query("DELETE FROM decisions WHERE ts_ms < ?1")
                .bind(cutoff)
                .execute(ledger.pool())
                .await
                .unwrap();
        }
    })
    .await;

    let batched = std::sync::Arc::new(ledger("batched").await);
    seed_decisions(&batched, BACKLOG, 10).await;
    let in_batches = override_writes_during(batched.clone(), {
        let ledger = batched.clone();
        async move {
            ledger
                .prune_expired(RETENTION_DAYS, 2_000, Duration::from_millis(5))
                .await;
        }
    })
    .await;
    assert_eq!(
        decisions_total(&batched).await,
        10,
        "the backlog was deleted"
    );

    // The control is printed for the record and not asserted: how many writes a single statement lets through
    // depends on the machine (67 on a fast CI runner, 3 on a laptop). That the prune is in batches is shown
    // deterministically above, by the statements it takes; this checks that writes do get through meanwhile.
    println!(
        "override writes while the backlog was deleted: one statement: {} completed, slowest {:?}; batches: {} completed, slowest {:?}",
        one_statement.during_prune,
        one_statement.slowest,
        in_batches.during_prune,
        in_batches.slowest
    );
    assert!(
        in_batches.during_prune >= 5,
        "writes should get through between batches: {} completed during the prune",
        in_batches.during_prune
    );
    assert!(
        in_batches.slowest < Duration::from_secs(1),
        "{:?}",
        in_batches.slowest
    );
}
