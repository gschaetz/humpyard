//! Routing decisions: one row per inference request (ADR 0013). Routing metadata only; never a
//! prompt or a response. Written by the ledger's asynchronous writer and read per key.

use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

use super::LedgerError;
use crate::num::{from_i64, to_i64};

/// Longest error text (and client-supplied model name) kept on a decision.
pub const MESSAGE_LIMIT: usize = 200;

/// One upstream attempt made while serving a request, in the order they happened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attempt {
    pub target: String,
    pub provider: String,
    pub model: String,
    /// `ok`, or the failure class (`rate_limit`, `timeout`, `server_error`, ...).
    pub outcome: String,
    pub ms: u64,
    /// The target's class label, when it has one (filled in when the decision is recorded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class: Option<String>,
    /// The retry time the endpoint stated for this failure, in milliseconds, when it was honored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<u64>,
}

/// The order a `fastest` route used for one request, and why its first target was first (ADR 0020).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ranking {
    pub order: Vec<String>,
    /// `fastest` or `explore`.
    pub reason: String,
}

/// A runtime override that took at least one candidate out of a request's plan (`add-runtime-overrides`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppliedOverride {
    pub id: i64,
    /// `disable_target`, `drain_provider` or `pin_route`.
    pub kind: String,
    pub subject: String,
}

/// A target removed for capability, and why: `tools`, `vision` or `context`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Incapable {
    pub target: String,
    pub reason: String,
}

/// What the router decided for one request, and how it went.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Decision {
    pub request_id: String,
    pub ts_ms: i64,
    pub key_id: Option<String>,
    pub session_id: Option<String>,
    /// Client protocol: `openai_chat`, `openai_responses` or `anthropic_messages`.
    pub format: String,
    pub requested_model: String,
    /// The route that served (or would have). `None` for requests refused before routing.
    pub route: Option<String>,
    pub rule: Option<String>,
    /// `selector` or `requested_model`.
    pub source: Option<String>,
    /// Rules that matched but were skipped because the key may not use their route.
    pub skipped_rules: Vec<String>,
    pub budget: Option<String>,
    pub eligible_targets: Vec<String>,
    /// Targets the route's `allowed_classes` removed before routing (ADR 0017).
    pub excluded_targets: Vec<String>,
    /// Targets skipped for this request because every endpoint was cooling down and another target
    /// remained (ADR 0018).
    pub cold_targets: Vec<String>,
    /// Targets removed because none of their endpoints could serve the request (ADR 0019).
    pub incapable_targets: Vec<Incapable>,
    /// The runtime overrides that removed a candidate from this request's plan (`add-runtime-overrides`).
    pub overrides: Vec<AppliedOverride>,
    /// For a `fastest` route, the order tried and why (ADR 0020).
    pub ranking: Option<Ranking>,
    pub stream: bool,
    pub prompt_tokens: Option<u64>,
    pub tools: bool,
    pub images: bool,
    pub attempts: Vec<Attempt>,
    pub status: u16,
    /// `ok`, or why not: `unauthorized`, `forbidden`, `unknown_model`, `bad_request`,
    /// `budget_exhausted`, `no_eligible_target`, `upstream_error`, `timeout`, `cancelled`, ...
    pub outcome: String,
    pub message: Option<String>,
    pub served_target: Option<String>,
    pub served_provider: Option<String>,
    /// Time from the request's arrival to the response (headers, for streams).
    pub latency_ms: u64,
}

impl Decision {
    /// Whether the attempts spanned more than one target class (for example a private target
    /// failing over to a public one). `None` when any attempt's class is unknown, or there were
    /// no attempts.
    #[must_use]
    pub fn crossed_classes(&self) -> Option<bool> {
        let classes: Vec<&str> = self
            .attempts
            .iter()
            .filter_map(|a| a.class.as_deref())
            .collect();
        if classes.is_empty() || classes.len() != self.attempts.len() {
            return None;
        }
        Some(classes.iter().any(|c| *c != classes[0]))
    }

    /// Truncates a text to 200 characters.
    #[must_use]
    pub fn clip(message: &str) -> String {
        message.chars().take(MESSAGE_LIMIT).collect()
    }
}

/// Filters for reading decisions. `key` is applied by the caller's identity, never by the client.
#[derive(Clone, Debug, Default)]
pub struct DecisionQuery {
    pub key: Option<String>,
    pub route: Option<String>,
    pub rule: Option<String>,
    pub outcome: Option<String>,
    pub request_id: Option<String>,
    pub since_ms: Option<i64>,
    pub limit: u32,
}

fn json_list<T: Serialize>(items: &[T]) -> String {
    serde_json::to_string(items).unwrap_or_else(|_| "[]".to_string())
}

/// v4: the targets a strict route's class list excluded. Old rows read as none.
pub(super) async fn migrate_v4(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let has: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('decisions') WHERE name = 'excluded_targets'",
    )
    .fetch_one(pool)
    .await?;
    if has == 0 {
        sqlx::query("ALTER TABLE decisions ADD COLUMN excluded_targets TEXT")
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// v6: the targets removed for capability. Old rows read as none.
pub(super) async fn migrate_v6(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let has: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('decisions') WHERE name = 'incapable_targets'",
    )
    .fetch_one(pool)
    .await?;
    if has == 0 {
        sqlx::query("ALTER TABLE decisions ADD COLUMN incapable_targets TEXT")
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// v10: the runtime overrides that applied to a request. Old rows read as none.
pub(super) async fn migrate_v10(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let has: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('decisions') WHERE name = 'overrides'",
    )
    .fetch_one(pool)
    .await?;
    if has == 0 {
        sqlx::query("ALTER TABLE decisions ADD COLUMN overrides TEXT")
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// v8: the order a `fastest` route used. Old rows read as none.
pub(super) async fn migrate_v8(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let has: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('decisions') WHERE name = 'ranking'",
    )
    .fetch_one(pool)
    .await?;
    if has == 0 {
        sqlx::query("ALTER TABLE decisions ADD COLUMN ranking TEXT")
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// v5: the targets skipped for health. Old rows read as none.
pub(super) async fn migrate_v5(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let has: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('decisions') WHERE name = 'cold_targets'",
    )
    .fetch_one(pool)
    .await?;
    if has == 0 {
        sqlx::query("ALTER TABLE decisions ADD COLUMN cold_targets TEXT")
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub(super) async fn migrate_v2(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS decisions (\
         id INTEGER PRIMARY KEY AUTOINCREMENT, \
         request_id TEXT NOT NULL, \
         ts_ms INTEGER NOT NULL, \
         key_id TEXT, \
         session_id TEXT, \
         format TEXT NOT NULL, \
         requested_model TEXT NOT NULL, \
         route TEXT, \
         rule TEXT, \
         source TEXT, \
         skipped_rules TEXT NOT NULL, \
         budget TEXT, \
         eligible_targets TEXT NOT NULL, \
         stream INTEGER NOT NULL, \
         prompt_tokens INTEGER, \
         tools INTEGER NOT NULL, \
         images INTEGER NOT NULL, \
         attempts TEXT NOT NULL, \
         status INTEGER NOT NULL, \
         outcome TEXT NOT NULL, \
         message TEXT, \
         served_target TEXT, \
         served_provider TEXT, \
         latency_ms INTEGER NOT NULL)",
    )
    .execute(pool)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS decisions_ts ON decisions (ts_ms)")
        .execute(pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS decisions_key_ts ON decisions (key_id, ts_ms)")
        .execute(pool)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS decisions_request ON decisions (request_id)")
        .execute(pool)
        .await?;
    Ok(())
}

pub(super) async fn insert_batch(pool: &SqlitePool, batch: &[Decision]) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    for d in batch {
        sqlx::query(
            "INSERT INTO decisions (request_id, ts_ms, key_id, session_id, format, requested_model, \
             route, rule, source, skipped_rules, budget, eligible_targets, stream, prompt_tokens, \
             tools, images, attempts, status, outcome, message, served_target, served_provider, \
             latency_ms, excluded_targets, cold_targets, incapable_targets, ranking, overrides) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28)",
        )
        .bind(&d.request_id)
        .bind(d.ts_ms)
        .bind(&d.key_id)
        .bind(&d.session_id)
        .bind(&d.format)
        .bind(Decision::clip(&d.requested_model))
        .bind(&d.route)
        .bind(&d.rule)
        .bind(&d.source)
        .bind(json_list(&d.skipped_rules))
        .bind(&d.budget)
        .bind(json_list(&d.eligible_targets))
        .bind(d.stream)
        .bind(d.prompt_tokens.map(to_i64))
        .bind(d.tools)
        .bind(d.images)
        .bind(json_list(&d.attempts))
        .bind(i64::from(d.status))
        .bind(&d.outcome)
        .bind(d.message.as_deref().map(Decision::clip))
        .bind(&d.served_target)
        .bind(&d.served_provider)
        .bind(to_i64(d.latency_ms))
        .bind(json_list(&d.excluded_targets))
        .bind(json_list(&d.cold_targets))
        .bind(json_list(&d.incapable_targets))
        .bind(d.ranking.as_ref().and_then(|r| serde_json::to_string(r).ok()))
        .bind(json_list(&d.overrides))
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await
}

/// Deletes decisions older than `cutoff_ms`.
pub(super) async fn prune(pool: &SqlitePool, cutoff_ms: i64) -> Result<u64, sqlx::Error> {
    Ok(sqlx::query("DELETE FROM decisions WHERE ts_ms < ?1")
        .bind(cutoff_ms)
        .execute(pool)
        .await?
        .rows_affected())
}

pub(super) async fn query(
    pool: &SqlitePool,
    q: &DecisionQuery,
) -> Result<Vec<Decision>, LedgerError> {
    // Each filter is `(?n IS NULL OR column = ?n)`, so one prepared statement serves all queries.
    let rows = sqlx::query(
        "SELECT * FROM decisions WHERE (?1 IS NULL OR key_id = ?1) AND (?2 IS NULL OR route = ?2) \
         AND (?3 IS NULL OR rule = ?3) AND (?4 IS NULL OR outcome = ?4) \
         AND (?5 IS NULL OR request_id = ?5) AND (?6 IS NULL OR ts_ms >= ?6) \
         ORDER BY ts_ms DESC, id DESC LIMIT ?7",
    )
    .bind(&q.key)
    .bind(&q.route)
    .bind(&q.rule)
    .bind(&q.outcome)
    .bind(&q.request_id)
    .bind(q.since_ms)
    .bind(i64::from(q.limit))
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Decision {
            request_id: row.get("request_id"),
            ts_ms: row.get("ts_ms"),
            key_id: row.get("key_id"),
            session_id: row.get("session_id"),
            format: row.get("format"),
            requested_model: row.get("requested_model"),
            route: row.get("route"),
            rule: row.get("rule"),
            source: row.get("source"),
            skipped_rules: serde_json::from_str(&row.get::<String, _>("skipped_rules"))
                .unwrap_or_default(),
            budget: row.get("budget"),
            eligible_targets: serde_json::from_str(&row.get::<String, _>("eligible_targets"))
                .unwrap_or_default(),
            stream: row.get("stream"),
            prompt_tokens: row.get::<Option<i64>, _>("prompt_tokens").map(from_i64),
            tools: row.get("tools"),
            images: row.get("images"),
            attempts: serde_json::from_str(&row.get::<String, _>("attempts")).unwrap_or_default(),
            status: u16::try_from(row.get::<i64, _>("status")).unwrap_or(0),
            outcome: row.get("outcome"),
            message: row.get("message"),
            served_target: row.get("served_target"),
            served_provider: row.get("served_provider"),
            latency_ms: from_i64(row.get::<i64, _>("latency_ms")),
            excluded_targets: row
                .get::<Option<String>, _>("excluded_targets")
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default(),
            cold_targets: row
                .get::<Option<String>, _>("cold_targets")
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default(),
            incapable_targets: row
                .get::<Option<String>, _>("incapable_targets")
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default(),
            ranking: row
                .get::<Option<String>, _>("ranking")
                .and_then(|json| serde_json::from_str(&json).ok()),
            overrides: row
                .get::<Option<String>, _>("overrides")
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default(),
        })
        .collect())
}
