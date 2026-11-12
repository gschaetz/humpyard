//! Statistics over time (ADR 0035): the same counts as [`super::stats`], bucketed by time. A series is read from the
//! decision log at query time, through the same query, row cap and counting function as the totals, so the two
//! cannot disagree. Read-only.

use std::collections::HashMap;

use sqlx::{Row, SqlitePool};

use super::LedgerError;
use super::stats::{Accumulator, StatsQuery, accumulate, decision_rows, percentile};
use crate::num::from_i64;

/// The most buckets a series holds. The endpoint refuses a request for more; this keeps a caller of the ledger from
/// building an unbounded answer.
pub const MAX_BUCKETS: usize = 400;

/// What to read: the statistics query (window start, grouping, filters, row cap) plus the bucket width, the end of
/// the window and how many groups to keep.
#[derive(Clone, Debug)]
pub struct SeriesQuery {
    pub stats: StatsQuery,
    /// Bucket width in milliseconds; buckets start at whole multiples of it since the Unix epoch.
    pub interval_ms: i64,
    /// The end of the window: the last bucket is the one containing it.
    pub now_ms: i64,
    /// The most groups kept, busiest first.
    pub limit: usize,
}

/// One bucket of one group. A bucket with no request has zero counts and `None` for rates and percentiles, never 0.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bucket {
    pub start_ms: i64,
    pub requests: u64,
    pub attempts: u64,
    pub failures: u64,
    pub failure_rate: Option<f64>,
    pub fallbacks: u64,
    pub fallback_rate: Option<f64>,
    pub p50_ms: Option<u64>,
    pub p90_ms: Option<u64>,
    pub p99_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GroupSeries {
    pub group: String,
    /// Every bucket from the one containing the window start to the one containing `now_ms`.
    pub buckets: Vec<Bucket>,
}

#[derive(Clone, Debug, Default)]
pub struct Series {
    /// The busiest groups by requests, then by name.
    pub groups: Vec<GroupSeries>,
    /// Groups with requests in the window that the `limit` left out.
    pub omitted_groups: u64,
    /// The window held more decisions than the row cap, so the oldest were left out.
    pub truncated: bool,
    /// When truncated, the time of the oldest decision counted: buckets before it are not quiet, just unread.
    pub covered_since_ms: Option<i64>,
}

#[allow(clippy::cast_precision_loss)] // counts far below 2^52; this is a display ratio
fn rate(part: u64, whole: u64) -> Option<f64> {
    (whole > 0).then(|| part as f64 / whole as f64)
}

fn bucket(start_ms: i64, acc: Option<&mut Accumulator>) -> Bucket {
    let Some(a) = acc else {
        return Bucket {
            start_ms,
            ..Bucket::default()
        };
    };
    a.latencies.sort_unstable();
    let failures = a.failures();
    Bucket {
        start_ms,
        requests: a.requests,
        attempts: a.attempts,
        failures,
        failure_rate: rate(failures, a.attempts),
        fallbacks: a.fallbacks,
        fallback_rate: rate(a.fallbacks, a.requests),
        p50_ms: percentile(&a.latencies, 50),
        p90_ms: percentile(&a.latencies, 90),
        p99_ms: percentile(&a.latencies, 99),
    }
}

/// Reads the decisions, then buckets them on a blocking thread (ADR 0036): the fold parses up to `row_cap` rows of
/// JSON and must never hold a worker of the runtime that serves requests. Only this function is visible outside the
/// module.
pub(super) async fn compute(pool: &SqlitePool, q: &SeriesQuery) -> Result<Series, LedgerError> {
    let rows = decision_rows(pool, &q.stats).await?;
    let query = q.clone();
    super::off_runtime(move || fold(&rows, &query)).await
}

/// The synchronous, CPU-bound half of [`compute`].
fn fold(rows: &[sqlx::sqlite::SqliteRow], q: &SeriesQuery) -> Series {
    let cap = q.stats.row_cap as usize;
    let truncated = rows.len() > cap;
    let covered_since_ms = truncated
        .then(|| rows.get(cap.saturating_sub(1)))
        .flatten()
        .map(|row| row.get::<i64, _>("ts_ms"));

    // The busiest groups come from the totals, which count by the same rules as the buckets.
    let totals = accumulate(rows, &q.stats, None);
    let mut ranked: Vec<(&String, u64)> =
        totals.iter().map(|((g, _), a)| (g, a.requests)).collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let omitted_groups =
        from_i64(i64::try_from(ranked.len().saturating_sub(q.limit)).unwrap_or(i64::MAX));
    let kept: Vec<String> = ranked
        .into_iter()
        .take(q.limit)
        .map(|(g, _)| g.clone())
        .collect();

    let interval = q.interval_ms.max(1);
    let first = q.stats.since_ms.div_euclid(interval) * interval;
    let last = q.now_ms.div_euclid(interval) * interval;
    let starts: Vec<i64> = std::iter::successors(Some(first), |s| Some(s + interval))
        .take_while(|s| *s <= last)
        .take(MAX_BUCKETS)
        .collect();

    let mut counted: HashMap<(String, Option<i64>), Accumulator> =
        accumulate(rows, &q.stats, Some(interval));
    let groups = kept
        .into_iter()
        .map(|group| GroupSeries {
            buckets: starts
                .iter()
                .map(|start| bucket(*start, counted.get_mut(&(group.clone(), Some(*start)))))
                .collect(),
            group,
        })
        .collect();
    Series {
        groups,
        omitted_groups,
        truncated,
        covered_since_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::super::stats::GroupBy;
    use super::*;

    const MINUTE: i64 = 60_000;

    /// One decision: its time, route and the attempts as (target, outcome, ms).
    type Decision = (i64, &'static str, Vec<(&'static str, &'static str, u64)>);

    async fn pool(decisions: &[Decision]) -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE decisions (route TEXT, attempts TEXT, ts_ms INTEGER, key_id TEXT)",
        )
        .execute(&pool)
        .await
        .unwrap();
        for (ts, route, attempts) in decisions {
            let attempts: Vec<_> = attempts
                .iter()
                .map(|(target, outcome, ms)| {
                    serde_json::json!({"target": target, "provider": "p", "model": target, "outcome": outcome, "ms": ms})
                })
                .collect();
            sqlx::query("INSERT INTO decisions VALUES (?1, ?2, ?3, NULL)")
                .bind(route)
                .bind(serde_json::Value::Array(attempts).to_string())
                .bind(ts)
                .execute(&pool)
                .await
                .unwrap();
        }
        pool
    }

    fn query(since_ms: i64, now_ms: i64) -> SeriesQuery {
        SeriesQuery {
            stats: StatsQuery {
                key: None,
                since_ms,
                group_by: GroupBy::Target,
                route: None,
                target: None,
                row_cap: 1000,
            },
            interval_ms: MINUTE,
            now_ms,
            limit: 8,
        }
    }

    fn group<'a>(series: &'a Series, name: &str) -> &'a GroupSeries {
        series.groups.iter().find(|g| g.group == name).unwrap()
    }

    #[tokio::test]
    async fn buckets_start_at_whole_multiples_of_the_interval_and_run_to_the_one_holding_now() {
        let p = pool(&[(7 * MINUTE + 5, "r", vec![("a", "ok", 10)])]).await;
        let mut q = query(5 * MINUTE + 30_000, 9 * MINUTE + 1);
        q.interval_ms = 2 * MINUTE;
        let series = compute(&p, &q).await.unwrap();
        let starts: Vec<i64> = group(&series, "a")
            .buckets
            .iter()
            .map(|b| b.start_ms)
            .collect();
        // The window starts inside the bucket at 4 minutes and now is inside the one at 8.
        assert_eq!(starts, vec![4 * MINUTE, 6 * MINUTE, 8 * MINUTE]);
        assert!(starts.iter().all(|s| s % (2 * MINUTE) == 0));
        // 7 min + 5 ms belongs to the bucket at 6 minutes.
        assert_eq!(group(&series, "a").buckets[1].requests, 1);
    }

    #[tokio::test]
    async fn a_quiet_bucket_is_present_with_zero_counts_and_no_rates() {
        let p = pool(&[
            (MINUTE, "r", vec![("a", "ok", 10)]),
            (3 * MINUTE, "r", vec![("a", "timeout", 90)]),
        ])
        .await;
        let series = compute(&p, &query(MINUTE, 3 * MINUTE)).await.unwrap();
        let buckets = &group(&series, "a").buckets;
        assert_eq!(buckets.len(), 3);
        let quiet = &buckets[1];
        assert_eq!((quiet.requests, quiet.attempts, quiet.failures), (0, 0, 0));
        assert_eq!(
            (
                quiet.failure_rate,
                quiet.fallback_rate,
                quiet.p50_ms,
                quiet.p90_ms,
                quiet.p99_ms
            ),
            (None, None, None, None, None)
        );
        assert_eq!(buckets[0].failure_rate, Some(0.0));
        assert_eq!(buckets[2].failure_rate, Some(1.0));
        assert_eq!(buckets[2].p99_ms, Some(90));
    }

    #[tokio::test]
    async fn only_the_busiest_groups_are_kept_and_the_rest_counted() {
        let mut decisions: Vec<Decision> = Vec::new();
        for (target, n) in [("a", 3), ("b", 2), ("c", 2), ("d", 1)] {
            for i in 0..n {
                decisions.push((MINUTE + i, "r", vec![(target, "ok", 5)]));
            }
        }
        let p = pool(&decisions).await;
        let mut q = query(MINUTE, MINUTE);
        q.limit = 2;
        let series = compute(&p, &q).await.unwrap();
        let names: Vec<&str> = series.groups.iter().map(|g| g.group.as_str()).collect();
        assert_eq!(names, vec!["a", "b"], "ties are broken by name");
        assert_eq!(series.omitted_groups, 2);
    }

    #[tokio::test]
    async fn a_window_cut_by_the_row_cap_says_what_it_covers() {
        let decisions: Vec<Decision> = (1..=5)
            .map(|i| (i * MINUTE, "r", vec![("a", "ok", 5)]))
            .collect();
        let p = pool(&decisions).await;
        let mut q = query(MINUTE, 5 * MINUTE);
        q.stats.row_cap = 3;
        let series = compute(&p, &q).await.unwrap();
        assert!(series.truncated);
        assert_eq!(
            series.covered_since_ms,
            Some(3 * MINUTE),
            "the oldest of the three newest rows"
        );
        let counted: u64 = group(&series, "a").buckets.iter().map(|b| b.requests).sum();
        assert_eq!(counted, 3);
        // A cap the window fits under is not truncation.
        q.stats.row_cap = 5;
        let whole = compute(&p, &q).await.unwrap();
        assert_eq!((whole.truncated, whole.covered_since_ms), (false, None));
    }

    #[tokio::test]
    async fn the_target_filter_counts_only_attempts_on_that_target() {
        let p = pool(&[(MINUTE, "r", vec![("a", "timeout", 80), ("b", "ok", 20)])]).await;
        let mut q = query(MINUTE, MINUTE);
        q.stats.target = Some("b".into());
        let series = compute(&p, &q).await.unwrap();
        assert_eq!(series.groups.len(), 1);
        let bucket = &group(&series, "b").buckets[0];
        assert_eq!(
            (bucket.requests, bucket.attempts, bucket.failures),
            (1, 1, 0)
        );
        assert_eq!(bucket.fallbacks, 1, "the request still made two attempts");
    }

    #[tokio::test]
    async fn groups_follow_the_chosen_grouping() {
        let p = pool(&[
            (MINUTE, "fast", vec![("a", "ok", 5)]),
            (MINUTE, "slow", vec![("a", "ok", 5), ("b", "ok", 5)]),
        ])
        .await;
        for (by, expected) in [
            (GroupBy::Route, vec!["fast", "slow"]),
            (GroupBy::Target, vec!["a", "b"]),
            (GroupBy::Endpoint, vec!["p/a", "p/b"]),
        ] {
            let mut q = query(MINUTE, MINUTE);
            q.stats.group_by = by;
            let series = compute(&p, &q).await.unwrap();
            let mut names: Vec<&str> = series.groups.iter().map(|g| g.group.as_str()).collect();
            names.sort_unstable();
            assert_eq!(names, expected, "{by:?}");
        }
    }

    #[tokio::test]
    async fn an_empty_window_has_no_groups() {
        let p = pool(&[]).await;
        let series = compute(&p, &query(MINUTE, 4 * MINUTE)).await.unwrap();
        assert_eq!(series.groups.len(), 0);
        assert_eq!((series.omitted_groups, series.truncated), (0, false));
    }

    mod props {
        use super::*;
        use proptest::prelude::*;

        fn decisions() -> impl Strategy<Value = Vec<Decision>> {
            let attempt = (
                prop::sample::select(vec!["a", "b", "c"]),
                prop::sample::select(vec!["ok", "timeout", "rate_limit"]),
                1u64..2000,
            );
            let decision = (
                0i64..(30 * MINUTE),
                prop::sample::select(vec!["r1", "r2"]),
                prop::collection::vec(attempt, 1..4),
            );
            prop::collection::vec(decision, 0..40)
        }

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(32))]

            #[test]
            fn each_groups_bucket_sums_equal_its_totals(rows in decisions(), by in 0usize..3, interval in 1i64..10) {
                let runtime = tokio::runtime::Runtime::new().unwrap();
                runtime.block_on(async {
                    let p = pool(&rows).await;
                    let mut q = query(0, 30 * MINUTE);
                    q.interval_ms = interval * MINUTE;
                    q.limit = 25;
                    q.stats.group_by = [GroupBy::Route, GroupBy::Target, GroupBy::Endpoint][by];
                    let series = compute(&p, &q).await.unwrap();
                    let totals = accumulate(
                        &decision_rows(&p, &q.stats).await.unwrap(),
                        &q.stats,
                        None,
                    );
                    assert_eq!(series.groups.len(), totals.len());
                    for g in &series.groups {
                        let t = &totals[&(g.group.clone(), None)];
                        let sum = |f: fn(&Bucket) -> u64| g.buckets.iter().map(f).sum::<u64>();
                        assert_eq!(sum(|b| b.requests), t.requests, "{}", g.group);
                        assert_eq!(sum(|b| b.attempts), t.attempts, "{}", g.group);
                        assert_eq!(sum(|b| b.failures), t.failures(), "{}", g.group);
                        assert_eq!(sum(|b| b.fallbacks), t.fallbacks, "{}", g.group);
                    }
                });
            }
        }
    }
}
