//! Runtime overrides and their audit trail (`add-runtime-overrides`). An override is an operator's
//! expiring instruction to stop using something; this module only stores and reads the rows. What an
//! override means for routing lives in `crate::overrides`. Names and reasons only; never a credential.

use serde::Serialize;
use sqlx::{Row, SqlitePool};

/// A stored override.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OverrideRecord {
    pub id: i64,
    /// `disable_target`, `drain_provider` or `pin_route`.
    pub kind: String,
    /// The target, provider or (for a pin) target the override names.
    pub subject: String,
    /// The route a `pin_route` applies to.
    pub route: Option<String>,
    pub reason: String,
    pub created_ms: i64,
    pub expires_ms: i64,
    /// The id of the key that created it, never the key.
    pub created_by: String,
    pub revoked_ms: Option<i64>,
}

/// One line of an override's history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OverrideAudit {
    pub override_id: i64,
    /// `created`, `revoked` or `expired`.
    pub event: String,
    pub at_ms: i64,
    /// The id of the key that acted; none for an expiry.
    pub by: Option<String>,
}

/// An override to store.
#[derive(Clone, Debug)]
pub struct NewOverride {
    pub kind: String,
    pub subject: String,
    pub route: Option<String>,
    pub reason: String,
    pub created_ms: i64,
    pub expires_ms: i64,
    pub created_by: String,
}

/// A handle on the override tables: the ledger's pool, cheap to clone, so the override store can outlive
/// the borrow of the `Ledger` that owns the writer. All SQL stays in this module.
#[derive(Clone)]
pub struct OverrideDb {
    pool: SqlitePool,
}

impl OverrideDb {
    pub(super) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Stores an override with its `created` audit row; returns its id. Written at once, not queued: the
    /// caller needs the id and needs to know it is stored.
    ///
    /// # Errors
    /// The database refuses the write.
    pub async fn create(&self, new: &NewOverride) -> Result<i64, super::LedgerError> {
        Ok(insert(&self.pool, new).await?)
    }

    /// Revokes an override that is still in force, with an audit row; false if there was none.
    ///
    /// # Errors
    /// The database refuses the write.
    pub async fn revoke(&self, id: i64, at_ms: i64, by: &str) -> Result<bool, super::LedgerError> {
        Ok(revoke(&self.pool, id, at_ms, by).await?)
    }

    /// Writes the `expired` audit row for each override that ran out before `now_ms`, once; returns their ids.
    ///
    /// # Errors
    /// The database refuses the write.
    pub async fn note_expired(&self, now_ms: i64) -> Result<Vec<i64>, super::LedgerError> {
        Ok(note_expired(&self.pool, now_ms).await?)
    }

    /// The overrides in force at `now_ms`.
    ///
    /// # Errors
    /// The database refuses the read.
    pub async fn active(&self, now_ms: i64) -> Result<Vec<OverrideRecord>, super::LedgerError> {
        Ok(active(&self.pool, now_ms).await?)
    }

    /// Overrides created at or after `since_ms`, in force or ended, newest first, at most `limit`.
    ///
    /// # Errors
    /// The database refuses the read.
    pub async fn recent(
        &self,
        since_ms: i64,
        limit: i64,
    ) -> Result<Vec<OverrideRecord>, super::LedgerError> {
        Ok(recent(&self.pool, since_ms, limit).await?)
    }

    /// The history of one override, oldest first.
    ///
    /// # Errors
    /// The database refuses the read.
    pub async fn audit(&self, id: i64) -> Result<Vec<OverrideAudit>, super::LedgerError> {
        Ok(audit(&self.pool, id).await?)
    }
}

pub(super) async fn migrate_v9(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS overrides (\
         id INTEGER PRIMARY KEY AUTOINCREMENT, \
         kind TEXT NOT NULL, \
         subject TEXT NOT NULL, \
         route TEXT, \
         reason TEXT NOT NULL, \
         created_ms INTEGER NOT NULL, \
         expires_ms INTEGER NOT NULL, \
         created_by TEXT NOT NULL, \
         revoked_ms INTEGER)",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS overrides_expires ON overrides (expires_ms)")
        .execute(pool)
        .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS override_audit (\
         id INTEGER PRIMARY KEY AUTOINCREMENT, \
         override_id INTEGER NOT NULL, \
         event TEXT NOT NULL, \
         at_ms INTEGER NOT NULL, \
         by_key TEXT)",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS override_audit_override ON override_audit (override_id)",
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Stores an override and its `created` audit row together; returns its id.
pub(super) async fn insert(pool: &SqlitePool, new: &NewOverride) -> Result<i64, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let id = sqlx::query(
        "INSERT INTO overrides (kind, subject, route, reason, created_ms, expires_ms, created_by) \
         VALUES (?1,?2,?3,?4,?5,?6,?7)",
    )
    .bind(&new.kind)
    .bind(&new.subject)
    .bind(&new.route)
    .bind(&new.reason)
    .bind(new.created_ms)
    .bind(new.expires_ms)
    .bind(&new.created_by)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();
    sqlx::query(
        "INSERT INTO override_audit (override_id, event, at_ms, by_key) VALUES (?1,'created',?2,?3)",
    )
    .bind(id)
    .bind(new.created_ms)
    .bind(&new.created_by)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// Revokes an override that is still in force at `at_ms`, with its `revoked` audit row. Returns whether
/// there was one: an unknown, already revoked or already expired override is not one.
pub(super) async fn revoke(
    pool: &SqlitePool,
    id: i64,
    at_ms: i64,
    by: &str,
) -> Result<bool, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let changed = sqlx::query(
        "UPDATE overrides SET revoked_ms = ?2 \
         WHERE id = ?1 AND revoked_ms IS NULL AND expires_ms > ?2",
    )
    .bind(id)
    .bind(at_ms)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed > 0 {
        sqlx::query(
            "INSERT INTO override_audit (override_id, event, at_ms, by_key) VALUES (?1,'revoked',?2,?3)",
        )
        .bind(id)
        .bind(at_ms)
        .bind(by)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(changed > 0)
}

/// Writes an `expired` audit row, once, for every override that ran out before `now_ms` and was not
/// revoked. The row is stamped with the time it expired, not the time it was noticed. Returns their ids.
pub(super) async fn note_expired(pool: &SqlitePool, now_ms: i64) -> Result<Vec<i64>, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let rows = sqlx::query(
        "SELECT id, expires_ms FROM overrides o \
         WHERE revoked_ms IS NULL AND expires_ms <= ?1 \
         AND NOT EXISTS (SELECT 1 FROM override_audit a WHERE a.override_id = o.id AND a.event = 'expired') \
         ORDER BY id",
    )
    .bind(now_ms)
    .fetch_all(&mut *tx)
    .await?;
    let mut ids = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row.try_get("id")?;
        let at: i64 = row.try_get("expires_ms")?;
        sqlx::query(
            "INSERT INTO override_audit (override_id, event, at_ms, by_key) VALUES (?1,'expired',?2,NULL)",
        )
        .bind(id)
        .bind(at)
        .execute(&mut *tx)
        .await?;
        ids.push(id);
    }
    tx.commit().await?;
    Ok(ids)
}

/// Overrides in force at `now_ms`: not revoked and not yet expired.
pub(super) async fn active(
    pool: &SqlitePool,
    now_ms: i64,
) -> Result<Vec<OverrideRecord>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, kind, subject, route, reason, created_ms, expires_ms, created_by, revoked_ms \
         FROM overrides WHERE revoked_ms IS NULL AND expires_ms > ?1 ORDER BY id",
    )
    .bind(now_ms)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_record).collect()
}

/// The history of one override, oldest first.
pub(super) async fn audit(
    pool: &SqlitePool,
    override_id: i64,
) -> Result<Vec<OverrideAudit>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT override_id, event, at_ms, by_key FROM override_audit WHERE override_id = ?1 ORDER BY id",
    )
    .bind(override_id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|r| {
            Ok(OverrideAudit {
                override_id: r.try_get("override_id")?,
                event: r.try_get("event")?,
                at_ms: r.try_get("at_ms")?,
                by: r.try_get("by_key")?,
            })
        })
        .collect()
}

/// Overrides created at or after `since_ms`, in force or ended, newest first, at most `limit`.
pub(super) async fn recent(
    pool: &SqlitePool,
    since_ms: i64,
    limit: i64,
) -> Result<Vec<OverrideRecord>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, kind, subject, route, reason, created_ms, expires_ms, created_by, revoked_ms \
         FROM overrides WHERE created_ms >= ?1 ORDER BY id DESC LIMIT ?2",
    )
    .bind(since_ms)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_record).collect()
}

fn row_to_record(r: &sqlx::sqlite::SqliteRow) -> Result<OverrideRecord, sqlx::Error> {
    Ok(OverrideRecord {
        id: r.try_get("id")?,
        kind: r.try_get("kind")?,
        subject: r.try_get("subject")?,
        route: r.try_get("route")?,
        reason: r.try_get("reason")?,
        created_ms: r.try_get("created_ms")?,
        expires_ms: r.try_get("expires_ms")?,
        created_by: r.try_get("created_by")?,
        revoked_ms: r.try_get("revoked_ms")?,
    })
}

/// Deletes audit rows older than `before_ms`, and the overrides that ended before it. An override still in
/// force is never deleted.
pub(super) async fn prune(pool: &SqlitePool, before_ms: i64) -> Result<u64, sqlx::Error> {
    let audit = sqlx::query("DELETE FROM override_audit WHERE at_ms < ?1")
        .bind(before_ms)
        .execute(pool)
        .await?
        .rows_affected();
    let ended = sqlx::query("DELETE FROM overrides WHERE COALESCE(revoked_ms, expires_ms) < ?1")
        .bind(before_ms)
        .execute(pool)
        .await?
        .rows_affected();
    Ok(audit + ended)
}
