//! Provider quota samples: how much of a provider's plan was used at a moment in time (ADR 0015).
//! Written by the ledger's asynchronous writer, read to compare a plan's percent with the cost
//! of the calls that were made. Numbers and short status text only; never a credential.

use serde::Serialize;
use sqlx::{Row, SqlitePool};

use crate::num::from_i64;

/// One usage window of one provider at one moment.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct QuotaSample {
    pub ts_ms: i64,
    pub provider: String,
    /// `rolling`, `weekly` or `monthly`.
    pub window: String,
    pub percent: f64,
    pub status: String,
    pub resets_at: Option<String>,
}

/// A stored sample, with the cost of the provider's calls since the previous sample of the same
/// window (micro-USD, from the usage rows; `None` for the first sample read).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct QuotaPoint {
    #[serde(flatten)]
    pub sample: QuotaSample,
    pub cost_micro_usd_since_previous: Option<u64>,
}

/// Which samples to read.
#[derive(Clone, Debug)]
pub struct QuotaQuery {
    pub since_ms: i64,
    pub provider: Option<String>,
    pub window: Option<String>,
    pub row_cap: u32,
}

pub(super) async fn migrate_v3(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS quota_samples (\
         id INTEGER PRIMARY KEY AUTOINCREMENT, \
         ts_ms INTEGER NOT NULL, \
         provider TEXT NOT NULL, \
         window_name TEXT NOT NULL, \
         percent REAL NOT NULL, \
         status TEXT NOT NULL, \
         resets_at TEXT)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS quota_samples_provider_ts ON quota_samples (provider, ts_ms)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub(super) async fn insert_batch(
    pool: &SqlitePool,
    batch: &[QuotaSample],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for s in batch {
        sqlx::query(
            "INSERT INTO quota_samples (ts_ms, provider, window_name, percent, status, resets_at) \
             VALUES (?1,?2,?3,?4,?5,?6)",
        )
        .bind(s.ts_ms)
        .bind(&s.provider)
        .bind(&s.window)
        .bind(s.percent)
        .bind(&s.status)
        .bind(&s.resets_at)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

pub(super) async fn prune(
    pool: &SqlitePool,
    before_ms: i64,
    batching: super::Batching,
) -> Result<super::Pruned, sqlx::Error> {
    super::delete_in_batches(pool, "quota_samples", "ts_ms < ?1", before_ms, batching).await
}

/// Samples matching `query`, oldest first, newest `row_cap` of them. Each carries the cost of the
/// provider's usage rows between the previous sample of its window and itself.
pub(super) async fn history(
    pool: &SqlitePool,
    query: &QuotaQuery,
) -> Result<Vec<QuotaPoint>, sqlx::Error> {
    // Optional filters are added as clauses with real binds, so no NULL is ever bound.
    let mut sql = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
        "SELECT ts_ms, provider, window_name, percent, status, resets_at, cost FROM (\
           SELECT s.ts_ms, s.id, s.provider, s.window_name, s.percent, s.status, s.resets_at, \
             CASE WHEN s.prev IS NULL THEN NULL ELSE \
               (SELECT COALESCE(SUM(u.cost_micro_usd), 0) FROM usage u \
                WHERE u.provider = s.provider AND u.ts_ms > s.prev AND u.ts_ms <= s.ts_ms) END AS cost \
           FROM (SELECT *, LAG(ts_ms) OVER (PARTITION BY provider, window_name ORDER BY ts_ms) AS prev \
                 FROM quota_samples WHERE ts_ms >= ",
    );
    sql.push_bind(query.since_ms);
    if let Some(provider) = &query.provider {
        sql.push(" AND provider = ").push_bind(provider);
    }
    if let Some(window) = &query.window {
        sql.push(" AND window_name = ").push_bind(window);
    }
    sql.push(") s ORDER BY s.ts_ms DESC, s.id DESC LIMIT ")
        .push_bind(i64::from(query.row_cap))
        .push(") ORDER BY ts_ms ASC, id ASC");
    let rows = sql.build().fetch_all(pool).await?;
    rows.iter()
        .map(|row| {
            let cost: Option<i64> = row.try_get("cost")?;
            Ok(QuotaPoint {
                sample: QuotaSample {
                    ts_ms: row.try_get("ts_ms")?,
                    provider: row.try_get("provider")?,
                    window: row.try_get("window_name")?,
                    percent: row.try_get("percent")?,
                    status: row.try_get("status")?,
                    resets_at: row.try_get("resets_at")?,
                },
                cost_micro_usd_since_previous: cost.map(from_i64),
            })
        })
        .collect()
}
