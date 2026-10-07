//! Usage ledger: one row per upstream call, written by a single background task so the request
//! path never waits on the database.

use std::collections::HashMap;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use sqlx::Row;

use crate::num::{from_i64, to_i64};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteSynchronous,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

const SCHEMA_VERSION: i64 = 1;
const BATCH: usize = 100;
const DEFAULT_CAPACITY: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The call whose response went to the client.
    Answer,
    /// A routing-time call (classifier or judge) the client never sees.
    Judge,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Answer => "answer",
            Self::Judge => "judge",
        }
    }
}

/// One upstream call.
#[derive(Clone, Debug)]
pub struct Entry {
    pub ts_ms: i64,
    pub key_id: Option<String>,
    pub session_id: Option<String>,
    pub route: String,
    pub target: String,
    pub provider: String,
    pub model: String,
    pub kind: Kind,
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub cache_creation_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cost_micro_usd: u64,
    /// `ok`, `error` or `cancelled`.
    pub outcome: String,
    /// The provider reported no usage, so the token counts and cost are zero by default.
    pub usage_missing: bool,
}

impl Entry {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            + self.cached_input_tokens
            + self.cache_creation_tokens
            + self.output_tokens
            + self.reasoning_tokens
    }
}

/// Spend for one key over a period.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Spend {
    pub micro_usd: u64,
    pub tokens: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("cannot open ledger {path}: {source}")]
    Open { path: String, source: sqlx::Error },
    #[error("ledger query failed: {0}")]
    Query(#[from] sqlx::Error),
}

pub struct Ledger {
    tx: mpsc::Sender<Entry>,
    pool: SqlitePool,
    /// Entries dropped because the writer's queue was full.
    dropped: Arc<AtomicU64>,
    /// Entries the writer failed to store.
    failed: Arc<AtomicU64>,
    writer: Option<JoinHandle<()>>,
}

impl Ledger {
    pub async fn open(path: &Path) -> Result<Self, LedgerError> {
        Self::open_with_capacity(path, DEFAULT_CAPACITY).await
    }

    pub async fn open_with_capacity(path: &Path, capacity: usize) -> Result<Self, LedgerError> {
        let open_error = |source| LedgerError::Open {
            path: path.display().to_string(),
            source,
        };
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
            .map_err(open_error)?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal);
        let pool = SqlitePoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .map_err(open_error)?;
        migrate(&pool).await.map_err(open_error)?;

        let (tx, mut rx) = mpsc::channel::<Entry>(capacity.max(1));
        let failed = Arc::new(AtomicU64::new(0));
        let writer_pool = pool.clone();
        let writer_failed = failed.clone();
        let writer = tokio::spawn(async move {
            let mut batch = Vec::with_capacity(BATCH);
            while rx.recv_many(&mut batch, BATCH).await > 0 {
                if let Err(error) = insert_batch(&writer_pool, &batch).await {
                    writer_failed.fetch_add(
                        u64::try_from(batch.len()).unwrap_or(u64::MAX),
                        Ordering::Relaxed,
                    );
                    tracing::error!(%error, entries = batch.len(), "ledger write failed; entries lost");
                }
                batch.clear();
            }
        });
        Ok(Self {
            tx,
            pool,
            dropped: Arc::new(AtomicU64::new(0)),
            failed,
            writer: Some(writer),
        })
    }

    /// Queues an entry. Never waits: a full queue drops the entry and counts it.
    pub fn record(&self, entry: Entry) {
        if self.tx.try_send(entry).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            tracing::error!("ledger queue full or closed; entry dropped");
        }
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    pub fn failed(&self) -> u64 {
        self.failed.load(Ordering::Relaxed)
    }

    /// Spend per key from `since_ms` (inclusive, unix milliseconds) to now.
    pub async fn spend_since(&self, since_ms: i64) -> Result<HashMap<String, Spend>, LedgerError> {
        let rows = sqlx::query(
            "SELECT key_id, SUM(cost_micro_usd) AS cost, \
             SUM(input_tokens + cached_input_tokens + cache_creation_tokens + output_tokens + reasoning_tokens) AS tokens \
             FROM usage WHERE ts_ms >= ?1 AND key_id IS NOT NULL GROUP BY key_id",
        )
        .bind(since_ms)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| {
                (
                    row.get::<String, _>("key_id"),
                    Spend {
                        micro_usd: from_i64(row.get::<i64, _>("cost")),
                        tokens: from_i64(row.get::<i64, _>("tokens")),
                    },
                )
            })
            .collect())
    }

    /// Every entry from `since_ms` (inclusive), oldest first. For reports and tests.
    pub async fn entries_since(&self, since_ms: i64) -> Result<Vec<Entry>, LedgerError> {
        let rows = sqlx::query("SELECT * FROM usage WHERE ts_ms >= ?1 ORDER BY id")
            .bind(since_ms)
            .fetch_all(&self.pool)
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| Entry {
                ts_ms: row.get("ts_ms"),
                key_id: row.get("key_id"),
                session_id: row.get("session_id"),
                route: row.get("route"),
                target: row.get("target"),
                provider: row.get("provider"),
                model: row.get("model"),
                kind: if row.get::<String, _>("kind") == "answer" {
                    Kind::Answer
                } else {
                    Kind::Judge
                },
                input_tokens: from_i64(row.get::<i64, _>("input_tokens")),
                cached_input_tokens: from_i64(row.get::<i64, _>("cached_input_tokens")),
                cache_creation_tokens: from_i64(row.get::<i64, _>("cache_creation_tokens")),
                output_tokens: from_i64(row.get::<i64, _>("output_tokens")),
                reasoning_tokens: from_i64(row.get::<i64, _>("reasoning_tokens")),
                cost_micro_usd: from_i64(row.get::<i64, _>("cost_micro_usd")),
                outcome: row.get("outcome"),
                usage_missing: row.get("usage_missing"),
            })
            .collect())
    }

    /// Stops accepting entries and waits for queued ones to be written.
    pub async fn shutdown(mut self) {
        let Self { tx, writer, .. } = &mut self;
        // Dropping the only sender lets the writer drain its queue and exit.
        let (closed, _) = mpsc::channel(1);
        drop(std::mem::replace(tx, closed));
        if let Some(writer) = writer.take() {
            let _ = writer.await;
        }
    }

    /// Direct pool access for tests that need to break or inspect the database.
    #[doc(hidden)]
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

async fn migrate(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(pool)
        .await?;
    if version < 1 {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS usage (\
             id INTEGER PRIMARY KEY AUTOINCREMENT, \
             ts_ms INTEGER NOT NULL, \
             key_id TEXT, \
             session_id TEXT, \
             route TEXT NOT NULL, \
             target TEXT NOT NULL, \
             provider TEXT NOT NULL, \
             model TEXT NOT NULL, \
             kind TEXT NOT NULL, \
             input_tokens INTEGER NOT NULL, \
             cached_input_tokens INTEGER NOT NULL, \
             cache_creation_tokens INTEGER NOT NULL, \
             output_tokens INTEGER NOT NULL, \
             reasoning_tokens INTEGER NOT NULL, \
             cost_micro_usd INTEGER NOT NULL, \
             outcome TEXT NOT NULL, \
             usage_missing INTEGER NOT NULL)",
        )
        .execute(pool)
        .await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS usage_key_ts ON usage (key_id, ts_ms)")
            .execute(pool)
            .await?;
        // PRAGMA does not take bound parameters; the value is a compile-time constant.
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "PRAGMA user_version = {SCHEMA_VERSION}"
        )))
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn insert_batch(pool: &SqlitePool, batch: &[Entry]) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for e in batch {
        sqlx::query(
            "INSERT INTO usage (ts_ms, key_id, session_id, route, target, provider, model, kind, \
             input_tokens, cached_input_tokens, cache_creation_tokens, output_tokens, reasoning_tokens, \
             cost_micro_usd, outcome, usage_missing) \
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        )
        .bind(e.ts_ms)
        .bind(&e.key_id)
        .bind(&e.session_id)
        .bind(&e.route)
        .bind(&e.target)
        .bind(&e.provider)
        .bind(&e.model)
        .bind(e.kind.as_str())
        .bind(to_i64(e.input_tokens))
        .bind(to_i64(e.cached_input_tokens))
        .bind(to_i64(e.cache_creation_tokens))
        .bind(to_i64(e.output_tokens))
        .bind(to_i64(e.reasoning_tokens))
        .bind(to_i64(e.cost_micro_usd))
        .bind(&e.outcome)
        .bind(e.usage_missing)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: Option<&str>, ts_ms: i64, cost: u64, tokens: u64) -> Entry {
        Entry {
            ts_ms,
            key_id: key.map(String::from),
            session_id: Some("s".into()),
            route: "auto".into(),
            target: "fast".into(),
            provider: "mock".into(),
            model: "m".into(),
            kind: Kind::Answer,
            input_tokens: tokens,
            cached_input_tokens: 0,
            cache_creation_tokens: 0,
            output_tokens: 0,
            reasoning_tokens: 0,
            cost_micro_usd: cost,
            outcome: "ok".into(),
            usage_missing: false,
        }
    }

    async fn open(dir: &tempfile::TempDir) -> Ledger {
        Ledger::open(&dir.path().join("ledger.db")).await.unwrap()
    }

    #[tokio::test]
    async fn entries_are_written_and_summed_per_key() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = open(&dir).await;
        ledger.record(entry(Some("alice"), 1_000, 500, 10));
        ledger.record(entry(Some("alice"), 2_000, 250, 5));
        ledger.record(entry(Some("bob"), 2_000, 100, 1));
        ledger.record(entry(None, 2_000, 999, 99)); // open-mode traffic has no key
        ledger.shutdown().await;

        let ledger = open(&dir).await;
        let spend = ledger.spend_since(0).await.unwrap();
        assert_eq!(
            spend["alice"],
            Spend {
                micro_usd: 750,
                tokens: 15
            }
        );
        assert_eq!(
            spend["bob"],
            Spend {
                micro_usd: 100,
                tokens: 1
            }
        );
        assert_eq!(spend.len(), 2);
    }

    #[tokio::test]
    async fn spend_since_only_counts_the_period() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = open(&dir).await;
        ledger.record(entry(Some("alice"), 1_000, 300, 3)); // yesterday
        ledger.record(entry(Some("alice"), 90_000_000, 200, 2)); // today
        ledger.shutdown().await;
        let ledger = open(&dir).await;
        let today = ledger.spend_since(86_400_000).await.unwrap();
        assert_eq!(
            today["alice"],
            Spend {
                micro_usd: 200,
                tokens: 2
            }
        );
    }

    #[tokio::test]
    async fn restart_recovers_current_day_and_month_spend() {
        use crate::clock::{day_start_ms, month_start_ms};
        let now = 1_791_000_000_000_i64; // an arbitrary instant in October 2026
        let (day, month) = (day_start_ms(now), month_start_ms(now));
        let dir = tempfile::tempdir().unwrap();
        let ledger = open(&dir).await;
        ledger.record(entry(Some("alice"), month + 1, 1_000, 10)); // earlier this month
        ledger.record(entry(Some("alice"), day + 1, 400, 4)); // today
        ledger.record(entry(Some("alice"), month - 1, 9_999, 99)); // last month
        ledger.shutdown().await;

        let restarted = open(&dir).await;
        let today = restarted.spend_since(day).await.unwrap();
        let this_month = restarted.spend_since(month).await.unwrap();
        assert_eq!(
            today["alice"],
            Spend {
                micro_usd: 400,
                tokens: 4
            }
        );
        assert_eq!(
            this_month["alice"],
            Spend {
                micro_usd: 1_400,
                tokens: 14
            }
        );
    }

    #[tokio::test]
    async fn schema_is_versioned_and_reopening_is_harmless() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = open(&dir).await;
        let version: i64 = sqlx::query_scalar("PRAGMA user_version")
            .fetch_one(ledger.pool())
            .await
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        ledger.record(entry(Some("a"), 1, 1, 1));
        ledger.shutdown().await;
        let again = open(&dir).await;
        assert_eq!(again.spend_since(0).await.unwrap()["a"].micro_usd, 1);
    }

    #[tokio::test]
    async fn a_broken_database_never_blocks_or_fails_recording() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = open(&dir).await;
        sqlx::query("DROP TABLE usage")
            .execute(ledger.pool())
            .await
            .unwrap();
        for i in 0..50 {
            ledger.record(entry(Some("a"), i, 1, 1));
        }
        // Writes fail in the background; recording itself kept returning immediately.
        for _ in 0..100 {
            if ledger.failed() >= 50 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(ledger.failed(), 50);
    }

    #[tokio::test]
    async fn a_full_queue_drops_and_counts_instead_of_waiting() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = Ledger::open_with_capacity(&dir.path().join("l.db"), 2)
            .await
            .unwrap();
        let started = std::time::Instant::now();
        for i in 0..5_000 {
            ledger.record(entry(Some("a"), i, 1, 1));
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "record must not wait"
        );
        assert!(
            ledger.dropped() > 0,
            "a 2-slot queue cannot absorb 5,000 instant entries"
        );
    }

    #[tokio::test]
    async fn opening_an_unwritable_path_is_a_clear_error() {
        let error = Ledger::open(Path::new("/nonexistent-dir/x/ledger.db"))
            .await
            .err()
            .unwrap();
        assert!(error.to_string().contains("cannot open ledger"));
    }
}
