//! Routing statistics: summaries of how routes, targets and endpoints have actually performed,
//! computed on read from the decision log and the usage rows (ADR 0013). Read-only: nothing in
//! routing uses these numbers.

use std::collections::{BTreeMap, HashMap};

use sqlx::{Row, SqlitePool};

use super::LedgerError;
use super::decisions::Attempt;
use crate::num::from_i64;

/// What to group the statistics by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupBy {
    /// The route that served the request.
    Route,
    /// The target an attempt was made on.
    Target,
    /// The provider and model an attempt was made on, as `provider/model`.
    Endpoint,
    /// The actor the request's key names (`add-actors`); requests of a key with none are the group `(none)`.
    Actor,
    /// The person the request's key rolls up to: its actor's owner, or the human itself for a human's own key.
    Owner,
}

/// The group of requests that no actor (or owner) is recorded for.
pub const NO_ACTOR: &str = "(none)";

/// Which decisions to summarize. `key` is applied by the caller's identity, never by the client.
#[derive(Clone, Debug)]
pub struct StatsQuery {
    pub key: Option<String>,
    pub since_ms: i64,
    pub group_by: GroupBy,
    pub route: Option<String>,
    pub target: Option<String>,
    /// The most decisions read; a window with more is summarized from the newest ones.
    pub row_cap: u32,
}

/// Performance of one group over the window.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroupStats {
    pub group: String,
    /// Requests that touched the group.
    pub requests: u64,
    pub attempts: u64,
    /// Attempts by outcome class (`ok`, `rate_limit`, `timeout`, ...).
    pub outcomes: BTreeMap<String, u64>,
    /// Attempts that did not succeed.
    pub failures: u64,
    /// Requests in the group that needed more than one attempt.
    pub fallbacks: u64,
    /// Attempt latency percentiles in milliseconds (nearest rank); `None` without attempts.
    pub p50_ms: Option<u64>,
    pub p90_ms: Option<u64>,
    pub p99_ms: Option<u64>,
    /// Answered requests that carry usage (the denominator of the two averages below).
    pub answered: u64,
    pub avg_cost_micro_usd: Option<f64>,
    pub avg_tokens: Option<f64>,
    /// Token totals of the group's answered calls that reported usage (calls marked as missing
    /// usage are left out, so they cannot pull the cache ratio down).
    pub tokens: TokenTotals,
    /// How the group's answer calls fared on the four response signals (ADR 0030).
    pub signals: SignalCounts,
}

/// How many answer calls were observed and how many carried each response signal. A call whose signals are
/// unknown (written before schema v13, a judge call, not observed) is in neither figure, so a rate is a count
/// over `observed` and old history does not look like perfect answers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SignalCounts {
    pub observed: u64,
    pub empty_output: u64,
    pub truncated_output: u64,
    pub bad_tool_call: u64,
    pub cut_stream: u64,
}

/// Token totals split the way providers bill them: a call's prompt is `input + cached_input +
/// cache_creation`, where `input` excludes the cached part.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TokenTotals {
    pub input: u64,
    pub cached_input: u64,
    pub cache_creation: u64,
    pub output: u64,
}

impl TokenTotals {
    /// The prompt tokens: fresh input plus the cached and cache-creation parts.
    #[must_use]
    pub fn prompt(&self) -> u64 {
        self.input
            .saturating_add(self.cached_input)
            .saturating_add(self.cache_creation)
    }

    /// Cached input over prompt tokens, or `None` when there are no prompt tokens.
    #[must_use]
    pub fn cache_hit_ratio(&self) -> Option<f64> {
        (self.prompt() > 0).then(|| ratio(self.cached_input, self.prompt()))
    }
}

impl GroupStats {
    /// Share of attempts that failed, or `None` without attempts.
    #[must_use]
    pub fn failure_rate(&self) -> Option<f64> {
        (self.attempts > 0).then(|| ratio(self.failures, self.attempts))
    }
}

#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub groups: Vec<GroupStats>,
    /// The window held more decisions than `row_cap`, so older ones were left out.
    pub truncated: bool,
}

#[allow(clippy::cast_precision_loss)] // counts far below 2^52; this is a display ratio
fn ratio(part: u64, whole: u64) -> f64 {
    part as f64 / whole as f64
}

/// The nearest-rank percentile of an ascending-sorted list: the smallest value with at least `p`
/// percent of the values at or below it.
#[must_use]
pub fn percentile(sorted: &[u64], p: u8) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let n = sorted.len();
    let rank = (n * usize::from(p.clamp(1, 100))).div_ceil(100).max(1);
    sorted.get(rank - 1).copied()
}

/// What one request contributes to a group, before aggregation.
#[derive(Default)]
pub(super) struct Accumulator {
    pub(super) requests: u64,
    pub(super) attempts: u64,
    pub(super) outcomes: BTreeMap<String, u64>,
    pub(super) fallbacks: u64,
    pub(super) latencies: Vec<u64>,
}

impl Accumulator {
    /// Attempts that did not succeed: every outcome but `ok`.
    pub(super) fn failures(&self) -> u64 {
        self.outcomes
            .iter()
            .filter(|(k, _)| *k != "ok")
            .map(|(_, v)| *v)
            .sum()
    }
}

/// Reads the decisions and the usage sums in the window, then folds them into per-group statistics on a blocking
/// thread (ADR 0036): the fold parses up to `row_cap` rows of JSON and must never hold a worker of the runtime that
/// serves requests. Only this function is visible outside the module.
pub(super) async fn compute(pool: &SqlitePool, q: &StatsQuery) -> Result<Stats, LedgerError> {
    let rows = decision_rows(pool, q).await?;
    let costs = cost_by_group(pool, q).await?;
    let query = q.clone();
    super::off_runtime(move || fold(&rows, &costs, &query)).await
}

/// The newest decisions in the window (one more than the cap, so a cut window is seen), newest first.
pub(super) async fn decision_rows(
    pool: &SqlitePool,
    q: &StatsQuery,
) -> Result<Vec<sqlx::sqlite::SqliteRow>, LedgerError> {
    Ok(sqlx::query(
        "SELECT route, actor, owner, attempts, ts_ms FROM decisions WHERE ts_ms >= ?1 AND (?2 IS NULL OR key_id = ?2) \
         AND (?3 IS NULL OR route = ?3) AND route IS NOT NULL ORDER BY ts_ms DESC LIMIT ?4",
    )
    .bind(q.since_ms)
    .bind(&q.key)
    .bind(&q.route)
    .bind(i64::from(q.row_cap) + 1)
    .fetch_all(pool)
    .await?)
}

/// The synchronous, CPU-bound half of [`compute`]: counts each request once per group it touched, and turns the
/// accumulators and the usage sums into the groups.
fn fold(
    rows: &[sqlx::sqlite::SqliteRow],
    costs: &HashMap<String, UsageSums>,
    q: &StatsQuery,
) -> Stats {
    let truncated = rows.len() > q.row_cap as usize;
    let acc: HashMap<String, Accumulator> = accumulate(rows, q, None)
        .into_iter()
        .map(|((group, _), a)| (group, a))
        .collect();

    let mut groups: Vec<GroupStats> = acc
        .into_iter()
        .map(|(group, mut a)| {
            a.latencies.sort_unstable();
            let failures = a.failures();
            let sums = costs.get(&group).copied().unwrap_or_default();
            let (answered, cost, tokens) = (sums.answered, sums.cost, sums.tokens);
            GroupStats {
                p50_ms: percentile(&a.latencies, 50),
                p90_ms: percentile(&a.latencies, 90),
                p99_ms: percentile(&a.latencies, 99),
                requests: a.requests,
                attempts: a.attempts,
                outcomes: a.outcomes,
                failures,
                fallbacks: a.fallbacks,
                answered,
                avg_cost_micro_usd: (answered > 0).then(|| ratio(cost, answered)),
                avg_tokens: (answered > 0).then(|| ratio(tokens, answered)),
                tokens: sums.totals,
                signals: sums.signals,
                group,
            }
        })
        .collect();
    groups.sort_by(|a, b| {
        b.requests
            .cmp(&a.requests)
            .then_with(|| a.group.cmp(&b.group))
    });
    Stats { groups, truncated }
}

/// Counts the decisions into accumulators keyed by `(group, bucket)`: the one place the counting rules live. A
/// request counts once per group it touched, however many attempts it made there; it is a fallback when it made more
/// than one attempt; an attempt counts only when it is on the `target` filter. With no `bucket_ms` the bucket is
/// `None` and the accumulators are the group totals; with one, the bucket is the request's time floored to a multiple
/// of it, so a series and the totals cannot count differently.
pub(super) fn accumulate(
    rows: &[sqlx::sqlite::SqliteRow],
    q: &StatsQuery,
    bucket_ms: Option<i64>,
) -> HashMap<(String, Option<i64>), Accumulator> {
    let mut acc: HashMap<(String, Option<i64>), Accumulator> = HashMap::new();
    for row in rows.iter().take(q.row_cap as usize) {
        let attempts: Vec<Attempt> =
            serde_json::from_str(&row.get::<String, _>("attempts")).unwrap_or_default();
        let route: String = row.get("route");
        let bucket = bucket_ms.map(|width| {
            let ts_ms: i64 = row.get("ts_ms");
            ts_ms.div_euclid(width) * width
        });
        let mut touched: HashMap<String, usize> = HashMap::new();
        let in_scope: Vec<&Attempt> = attempts
            .iter()
            .filter(|a| q.target.as_ref().is_none_or(|t| &a.target == t))
            .collect();
        match q.group_by {
            GroupBy::Route | GroupBy::Actor | GroupBy::Owner => {
                // One group per request: its route, or the actor or owner it was made under.
                let group = match q.group_by {
                    GroupBy::Actor => row.get::<Option<String>, _>("actor"),
                    GroupBy::Owner => row.get::<Option<String>, _>("owner"),
                    _ => Some(route),
                }
                .unwrap_or_else(|| NO_ACTOR.to_string());
                let entry = acc.entry((group, bucket)).or_default();
                entry.requests += 1;
                entry.fallbacks += u64::from(attempts.len() > 1);
                for a in &in_scope {
                    add_attempt(entry, a);
                }
            }
            GroupBy::Target | GroupBy::Endpoint => {
                for a in &in_scope {
                    let group = if q.group_by == GroupBy::Target {
                        a.target.clone()
                    } else {
                        format!("{}/{}", a.provider, a.model)
                    };
                    *touched.entry(group.clone()).or_default() += 1;
                    add_attempt(acc.entry((group, bucket)).or_default(), a);
                }
                for (group, n) in touched {
                    let entry = acc.entry((group, bucket)).or_default();
                    entry.requests += 1;
                    entry.fallbacks += u64::from(n > 1 || attempts.len() > 1);
                }
            }
        }
    }
    acc
}

fn add_attempt(entry: &mut Accumulator, attempt: &Attempt) {
    entry.attempts += 1;
    *entry.outcomes.entry(attempt.outcome.clone()).or_default() += 1;
    entry.latencies.push(attempt.ms);
}

/// The calls whose four signals are all known (observed); the others are left out of every signal figure.
const KNOWN: &str = "empty_output IS NOT NULL AND truncated_output IS NOT NULL \
                     AND bad_tool_call IS NOT NULL AND cut_stream IS NOT NULL";

/// What the usage rows say about one group.
#[derive(Clone, Copy, Debug, Default)]
struct UsageSums {
    answered: u64,
    cost: u64,
    tokens: u64,
    totals: TokenTotals,
    signals: SignalCounts,
}

/// Answered requests, total cost and total tokens per group, and the token split of the calls that
/// reported usage, from the usage rows.
async fn cost_by_group(
    pool: &SqlitePool,
    q: &StatsQuery,
) -> Result<HashMap<String, UsageSums>, LedgerError> {
    let column = match q.group_by {
        GroupBy::Route => "route",
        GroupBy::Target => "target",
        GroupBy::Endpoint => "provider || '/' || model",
        // A usage row has no actor of its own; its decision (the same request id) does.
        GroupBy::Actor => {
            "COALESCE((SELECT d.actor FROM decisions d WHERE d.request_id = usage.request_id), '(none)')"
        }
        GroupBy::Owner => {
            "COALESCE((SELECT d.owner FROM decisions d WHERE d.request_id = usage.request_id), '(none)')"
        }
    };
    // `column` is one of the constants above and `KNOWN` a constant, never client input.
    let sql = format!(
        "SELECT {column} AS g, COUNT(*) AS answered, SUM(cost_micro_usd) AS cost, \
         SUM(input_tokens + cached_input_tokens + cache_creation_tokens + output_tokens) AS tokens, \
         SUM(CASE WHEN usage_missing = 0 THEN input_tokens ELSE 0 END) AS fresh, \
         SUM(CASE WHEN usage_missing = 0 THEN cached_input_tokens ELSE 0 END) AS cached, \
         SUM(CASE WHEN usage_missing = 0 THEN cache_creation_tokens ELSE 0 END) AS creation, \
         SUM(CASE WHEN usage_missing = 0 THEN output_tokens ELSE 0 END) AS produced, \
         SUM(CASE WHEN {KNOWN} THEN 1 ELSE 0 END) AS observed, \
         SUM(CASE WHEN {KNOWN} THEN empty_output ELSE 0 END) AS empty, \
         SUM(CASE WHEN {KNOWN} THEN truncated_output ELSE 0 END) AS truncated, \
         SUM(CASE WHEN {KNOWN} THEN bad_tool_call ELSE 0 END) AS bad_call, \
         SUM(CASE WHEN {KNOWN} THEN cut_stream ELSE 0 END) AS cut \
         FROM usage WHERE ts_ms >= ?1 AND kind = 'answer' AND (?2 IS NULL OR key_id = ?2) \
         AND (?3 IS NULL OR route = ?3) AND (?4 IS NULL OR target = ?4) GROUP BY g"
    );
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(q.since_ms)
        .bind(&q.key)
        .bind(&q.route)
        .bind(&q.target)
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.get::<String, _>("g"),
                UsageSums {
                    answered: from_i64(r.get::<i64, _>("answered")),
                    cost: from_i64(r.get::<i64, _>("cost")),
                    tokens: from_i64(r.get::<i64, _>("tokens")),
                    totals: TokenTotals {
                        input: from_i64(r.get::<i64, _>("fresh")),
                        cached_input: from_i64(r.get::<i64, _>("cached")),
                        cache_creation: from_i64(r.get::<i64, _>("creation")),
                        output: from_i64(r.get::<i64, _>("produced")),
                    },
                    signals: SignalCounts {
                        observed: from_i64(r.get::<i64, _>("observed")),
                        empty_output: from_i64(r.get::<i64, _>("empty")),
                        truncated_output: from_i64(r.get::<i64, _>("truncated")),
                        bad_tool_call: from_i64(r.get::<i64, _>("bad_call")),
                        cut_stream: from_i64(r.get::<i64, _>("cut")),
                    },
                },
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed window read through the same query the statistics use: three requests on two targets, one of them a
    /// fallback, two minutes apart, so a one-minute bucket separates them.
    async fn window() -> Vec<sqlx::sqlite::SqliteRow> {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE decisions (route TEXT, actor TEXT, owner TEXT, attempts TEXT, ts_ms INTEGER)")
            .execute(&pool)
            .await
            .unwrap();
        let attempt = |target: &str, outcome: &str, ms: u64| serde_json::json!({"target": target, "provider": "p", "model": target, "outcome": outcome, "ms": ms});
        let rows = [
            (60_000, vec![attempt("a", "ok", 100)]),
            (
                125_000,
                vec![attempt("a", "timeout", 900), attempt("b", "ok", 200)],
            ),
            (130_000, vec![attempt("b", "ok", 300)]),
        ];
        for (ts, attempts) in rows {
            sqlx::query("INSERT INTO decisions (route, attempts, ts_ms) VALUES ('r', ?1, ?2)")
                .bind(serde_json::Value::Array(attempts).to_string())
                .bind(ts)
                .execute(&pool)
                .await
                .unwrap();
        }
        sqlx::query("SELECT route, attempts, ts_ms FROM decisions ORDER BY ts_ms DESC")
            .fetch_all(&pool)
            .await
            .unwrap()
    }

    fn query() -> StatsQuery {
        StatsQuery {
            key: None,
            since_ms: 0,
            group_by: GroupBy::Target,
            route: None,
            target: None,
            row_cap: 100,
        }
    }

    #[tokio::test]
    async fn the_totals_of_a_fixed_window_are_the_counts_the_rules_give() {
        let acc = accumulate(&window().await, &query(), None);
        let a = &acc[&("a".to_string(), None)];
        assert_eq!((a.requests, a.attempts, a.fallbacks), (2, 2, 1));
        assert_eq!(a.outcomes.get("timeout"), Some(&1));
        let b = &acc[&("b".to_string(), None)];
        assert_eq!((b.requests, b.attempts, b.fallbacks), (2, 2, 1));
        assert_eq!(acc.len(), 2);
    }

    #[tokio::test]
    async fn buckets_are_floors_of_the_interval_and_add_up_to_the_totals() {
        let rows = window().await;
        let totals = accumulate(&rows, &query(), None);
        let buckets = accumulate(&rows, &query(), Some(60_000));
        let starts: std::collections::BTreeSet<i64> =
            buckets.keys().filter_map(|(_, b)| *b).collect();
        assert_eq!(
            starts.into_iter().collect::<Vec<_>>(),
            vec![60_000, 120_000],
            "125 s and 130 s share the bucket at 120 s"
        );
        for ((group, _), total) in &totals {
            let sum = |f: fn(&Accumulator) -> u64| -> u64 {
                buckets
                    .iter()
                    .filter(|((g, _), _)| g == group)
                    .map(|(_, a)| f(a))
                    .sum()
            };
            assert_eq!(sum(|a| a.requests), total.requests, "{group}");
            assert_eq!(sum(|a| a.attempts), total.attempts, "{group}");
            assert_eq!(sum(|a| a.fallbacks), total.fallbacks, "{group}");
        }
    }

    #[test]
    fn nearest_rank_percentiles_match_the_textbook_definition() {
        let v: Vec<u64> = (1..=10).map(|n| n * 10).collect(); // 10..100
        assert_eq!(percentile(&v, 50), Some(50));
        assert_eq!(percentile(&v, 90), Some(90));
        assert_eq!(percentile(&v, 99), Some(100));
        assert_eq!(percentile(&v, 100), Some(100));
        assert_eq!(percentile(&v, 1), Some(10));
        assert_eq!(percentile(&[7], 50), Some(7));
        assert_eq!(percentile(&[7], 99), Some(7));
        assert_eq!(percentile(&[], 50), None);
        let four = [10, 20, 30, 40];
        assert_eq!(percentile(&four, 50), Some(20), "rank ceil(0.5*4) = 2");
        assert_eq!(percentile(&four, 75), Some(30));
    }

    mod props {
        use proptest::prelude::*;

        use super::super::percentile;

        proptest! {
            #[test]
            fn a_percentile_is_one_of_the_values_and_never_decreases_with_p(
                mut values in proptest::collection::vec(0_u64..10_000, 1..200)
            ) {
                values.sort_unstable();
                let (p50, p90, p99) = (
                    percentile(&values, 50).unwrap(),
                    percentile(&values, 90).unwrap(),
                    percentile(&values, 99).unwrap(),
                );
                prop_assert!(values.contains(&p50) && values.contains(&p90) && values.contains(&p99));
                prop_assert!(p50 <= p90 && p90 <= p99);
                prop_assert!(p99 <= *values.last().unwrap());
            }
        }
    }
}
