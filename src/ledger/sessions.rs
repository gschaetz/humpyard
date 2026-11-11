//! Session analytics: how sessions spent, how much of their prompts came from cache, and what a switch
//! between endpoints abandoned. Computed on read from the usage rows (ADR 0013); read-only, and nothing
//! in routing uses it. A session is the opaque id a client sent; calls without one are counted, never
//! grouped.

use std::collections::{BTreeMap, HashMap};

use sqlx::{Row, SqlitePool};

use super::LedgerError;
use super::stats::TokenTotals;
use crate::num::from_i64;

/// Which usage rows to read. `key` is applied by the caller's identity, never by the client.
#[derive(Clone, Debug)]
pub struct SessionQuery {
    pub key: Option<String>,
    pub since_ms: i64,
    /// At most this many session-bearing usage rows are read (newest first).
    pub row_cap: u32,
}

/// One session in the window.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionStats {
    pub session: String,
    /// Answered calls.
    pub calls: u64,
    pub cost_micro_usd: u64,
    pub tokens: TokenTotals,
    /// The `provider/model` endpoints used, in order of first use.
    pub endpoints: Vec<String>,
    /// Consecutive answered calls on different endpoints.
    pub switches: u64,
    pub started_ms: i64,
    pub ended_ms: i64,
}

/// A change of endpoint between two consecutive answered calls of a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Switch {
    pub from: String,
    pub to: String,
    /// What the call before the switch left cached: its cached input plus cache-creation tokens, which
    /// the new endpoint must pay the fresh input price for.
    pub warm_tokens: u64,
}

/// Everything session views are built from.
#[derive(Clone, Debug, Default)]
pub struct SessionScan {
    pub sessions: Vec<SessionStats>,
    pub switches: Vec<Switch>,
    /// All answered calls in the window for the caller, with or without a session id.
    pub answered: u64,
    pub answered_with_session: u64,
    /// Their total cost, the denominator of "share of spend".
    pub total_cost_micro_usd: u64,
    /// The window held more session-bearing rows than `row_cap`, so older ones were left out.
    pub truncated: bool,
}

/// One answered call, read from a usage row.
struct Call {
    ts_ms: i64,
    id: i64,
    endpoint: String,
    cost: u64,
    usage_missing: bool,
    tokens: TokenTotals,
}

pub(super) async fn scan(pool: &SqlitePool, q: &SessionQuery) -> Result<SessionScan, LedgerError> {
    let totals = sqlx::query(
        "SELECT COUNT(*) AS answered, \
         COALESCE(SUM(CASE WHEN session_id IS NOT NULL THEN 1 ELSE 0 END), 0) AS with_session, \
         COALESCE(SUM(cost_micro_usd), 0) AS cost \
         FROM usage WHERE ts_ms >= ?1 AND kind = 'answer' AND (?2 IS NULL OR key_id = ?2)",
    )
    .bind(q.since_ms)
    .bind(&q.key)
    .fetch_one(pool)
    .await?;

    let rows = sqlx::query(
        "SELECT id, ts_ms, session_id, provider, model, input_tokens, cached_input_tokens, \
         cache_creation_tokens, output_tokens, cost_micro_usd, usage_missing \
         FROM usage WHERE ts_ms >= ?1 AND kind = 'answer' AND session_id IS NOT NULL \
         AND (?2 IS NULL OR key_id = ?2) ORDER BY ts_ms DESC, id DESC LIMIT ?3",
    )
    .bind(q.since_ms)
    .bind(&q.key)
    .bind(i64::from(q.row_cap) + 1)
    .fetch_all(pool)
    .await?;
    let cap = q.row_cap as usize;
    super::off_runtime(move || fold(&totals, &rows, cap)).await
}

/// The synchronous, CPU-bound half of [`scan`] (ADR 0036): groups the newest rows by session, puts each session's
/// calls in time order and summarizes them. Run on a blocking thread, and private to this module.
fn fold(
    totals: &sqlx::sqlite::SqliteRow,
    rows: &[sqlx::sqlite::SqliteRow],
    cap: usize,
) -> SessionScan {
    let truncated = rows.len() > cap;

    // Newest rows were read; group them by session and put each session's calls in time order.
    let mut by_session: HashMap<String, Vec<Call>> = HashMap::new();
    for row in rows.iter().take(cap) {
        by_session
            .entry(row.get("session_id"))
            .or_default()
            .push(Call {
                ts_ms: row.get("ts_ms"),
                id: row.get("id"),
                endpoint: format!(
                    "{}/{}",
                    row.get::<String, _>("provider"),
                    row.get::<String, _>("model")
                ),
                cost: from_i64(row.get::<i64, _>("cost_micro_usd")),
                usage_missing: row.get::<i64, _>("usage_missing") != 0,
                tokens: TokenTotals {
                    input: from_i64(row.get::<i64, _>("input_tokens")),
                    cached_input: from_i64(row.get::<i64, _>("cached_input_tokens")),
                    cache_creation: from_i64(row.get::<i64, _>("cache_creation_tokens")),
                    output: from_i64(row.get::<i64, _>("output_tokens")),
                },
            });
    }

    let mut sessions = Vec::new();
    let mut switches = Vec::new();
    // A BTreeMap keeps the output deterministic for sessions with equal figures.
    let ordered: BTreeMap<String, Vec<Call>> = by_session.into_iter().collect();
    for (session, mut calls) in ordered {
        calls.sort_by_key(|c| (c.ts_ms, c.id));
        let (stats, found) = summarize(session, &calls);
        sessions.push(stats);
        switches.extend(found);
    }

    SessionScan {
        sessions,
        switches,
        answered: from_i64(totals.get::<i64, _>("answered")),
        answered_with_session: from_i64(totals.get::<i64, _>("with_session")),
        total_cost_micro_usd: from_i64(totals.get::<i64, _>("cost")),
        truncated,
    }
}

/// A session's figures from its calls in time order, and the switches between them.
fn summarize(session: String, calls: &[Call]) -> (SessionStats, Vec<Switch>) {
    let mut tokens = TokenTotals::default();
    let mut cost = 0_u64;
    let mut endpoints: Vec<String> = Vec::new();
    let mut found = Vec::new();
    for (i, call) in calls.iter().enumerate() {
        cost = cost.saturating_add(call.cost);
        if !call.usage_missing {
            tokens.input = tokens.input.saturating_add(call.tokens.input);
            tokens.cached_input = tokens.cached_input.saturating_add(call.tokens.cached_input);
            tokens.cache_creation = tokens
                .cache_creation
                .saturating_add(call.tokens.cache_creation);
            tokens.output = tokens.output.saturating_add(call.tokens.output);
        }
        if !endpoints.contains(&call.endpoint) {
            endpoints.push(call.endpoint.clone());
        }
        if let Some(previous) = i.checked_sub(1).and_then(|p| calls.get(p))
            && previous.endpoint != call.endpoint
        {
            let warm = if previous.usage_missing {
                0
            } else {
                previous
                    .tokens
                    .cached_input
                    .saturating_add(previous.tokens.cache_creation)
            };
            found.push(Switch {
                from: previous.endpoint.clone(),
                to: call.endpoint.clone(),
                warm_tokens: warm,
            });
        }
    }
    let stats = SessionStats {
        session,
        calls: u64::try_from(calls.len()).unwrap_or(u64::MAX),
        cost_micro_usd: cost,
        tokens,
        endpoints,
        switches: u64::try_from(found.len()).unwrap_or(u64::MAX),
        started_ms: calls.first().map_or(0, |c| c.ts_ms),
        ended_ms: calls.last().map_or(0, |c| c.ts_ms),
    };
    (stats, found)
}
