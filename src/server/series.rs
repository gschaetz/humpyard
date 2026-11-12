//! `GET /admin/v1/stats/series`: the statistics of `GET /admin/v1/stats` over time (ADR 0035), a sequence of equal
//! buckets per group. Admin only (`read` scope), across keys with an optional `key` filter. Every bound is checked
//! before anything is read, and the read itself goes through the gate and the shared result of the heavy reads
//! (ADR 0036).

use std::collections::HashMap;

use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use switchyard_protocol::WireFormat;

use super::heavy::{Busy, Surface};
use super::stats::{DEFAULT_WINDOW_MS, ROW_CAP, longest_window, parse_window};
use super::{AppState, GatewayError};
use crate::ledger::{Bucket, GroupBy, Series, SeriesQuery, StatsQuery};

const MINUTE_MS: i64 = 60_000;
/// The intervals the server picks from when none is given, finest first.
const INTERVALS_MS: [i64; 9] = [
    MINUTE_MS,
    5 * MINUTE_MS,
    15 * MINUTE_MS,
    30 * MINUTE_MS,
    60 * MINUTE_MS,
    120 * MINUTE_MS,
    360 * MINUTE_MS,
    720 * MINUTE_MS,
    1_440 * MINUTE_MS,
];
/// The most buckets a default interval gives.
const DEFAULT_BUCKETS: i64 = 120;
/// The most buckets a window may be cut into (the ledger leaves room for one more, the partial first bucket).
const MAX_WINDOW_BUCKETS: i64 = 400;
const DEFAULT_LIMIT: usize = 8;
const MAX_LIMIT: usize = 25;
const PARAMETERS: [&str; 7] = [
    "window", "interval", "group_by", "route", "target", "key", "limit",
];

/// How many buckets of `interval_ms` (positive) cover `window_ms`, rounded up.
fn buckets_in(window_ms: i64, interval_ms: i64) -> i64 {
    window_ms.saturating_add(interval_ms - 1) / interval_ms
}

/// The checked parameters of one read.
struct Request {
    series: SeriesQuery,
    window_ms: i64,
}

/// Reads the parameters, refusing anything unknown or out of bounds with a message that names the parameter.
fn parse(
    params: &HashMap<String, String>,
    now_ms: i64,
    longest_ms: i64,
) -> Result<Request, String> {
    if let Some(name) = params.keys().find(|k| !PARAMETERS.contains(&k.as_str())) {
        return Err(format!("`{name}` is not a parameter of this endpoint"));
    }
    let window_ms = params
        .get("window")
        .map_or(Ok(DEFAULT_WINDOW_MS), |w| parse_window(w))?;
    if window_ms > longest_ms {
        return Err(format!(
            "`window` is longer than the {} days of decisions kept",
            longest_ms / 86_400_000
        ));
    }
    let interval_ms = match params.get("interval") {
        Some(text) => parse_window(text)
            .map_err(|_| format!("`interval` must look like 5m, 2h or 1d, not `{text}`"))?,
        None => INTERVALS_MS
            .into_iter()
            .find(|i| buckets_in(window_ms, *i) <= DEFAULT_BUCKETS)
            .unwrap_or(INTERVALS_MS[INTERVALS_MS.len() - 1]),
    };
    if interval_ms < MINUTE_MS {
        return Err("`interval` must be at least 1m".into());
    }
    let buckets = buckets_in(window_ms, interval_ms);
    if buckets > MAX_WINDOW_BUCKETS {
        return Err(format!(
            "`interval` would give {buckets} buckets for this `window`; at most {MAX_WINDOW_BUCKETS} are allowed, so use a longer interval"
        ));
    }
    let group_by = match params.get("group_by").map(String::as_str) {
        None | Some("route") => GroupBy::Route,
        Some("target") => GroupBy::Target,
        Some("endpoint") => GroupBy::Endpoint,
        Some(other) => {
            return Err(format!(
                "`group_by` must be route, target or endpoint, not `{other}`"
            ));
        }
    };
    let limit = match params.get("limit") {
        None => DEFAULT_LIMIT,
        Some(text) => text
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=MAX_LIMIT).contains(n))
            .ok_or_else(|| {
                format!("`limit` must be a number from 1 to {MAX_LIMIT}, not `{text}`")
            })?,
    };
    Ok(Request {
        window_ms,
        series: SeriesQuery {
            stats: StatsQuery {
                key: params.get("key").cloned(),
                since_ms: now_ms.saturating_sub(window_ms),
                group_by,
                route: params.get("route").cloned(),
                target: params.get("target").cloned(),
                row_cap: ROW_CAP,
            },
            interval_ms,
            now_ms,
            limit,
        },
    })
}

/// The series answer for the administrative API. A bad parameter is refused here, before the gate is touched.
pub(super) async fn series_view(
    state: &AppState,
    snap: &super::Snapshot,
    params: &HashMap<String, String>,
) -> Response {
    let format = WireFormat::OpenAiChat;
    let request = match parse(params, state.clock.now_ms(), longest_window(snap)) {
        Ok(request) => request,
        Err(message) => return GatewayError::BadRequest(message).into_response_for(format),
    };
    // `key` is both a filter and the caller part of the shared result's identity, as for the statistics.
    let key = params.get("key").cloned();
    state
        .heavy
        .run(
            Surface::Admin,
            "series",
            key.as_deref(),
            params,
            series_answer(state, &request),
        )
        .await
}

async fn series_answer(state: &AppState, request: &Request) -> Result<Response, Busy> {
    let format = WireFormat::OpenAiChat;
    let Some(ledger) = state.accounting.ledger() else {
        return Ok(GatewayError::ModelNotFound(
            "no ledger is configured, so statistics are not available".into(),
        )
        .into_response_for(format));
    };
    ledger.flush().await;
    let q = &request.series;
    match ledger.series(q).await {
        Ok(series) => {
            Ok(axum::Json(series_json(state.clock.now_ms(), request, &series)).into_response())
        }
        Err(error) if error.is_busy() => Err(Busy),
        Err(error) => Ok(GatewayError::Internal(error.to_string()).into_response_for(format)),
    }
}

fn group_by_name(by: GroupBy) -> &'static str {
    match by {
        GroupBy::Route => "route",
        GroupBy::Target => "target",
        GroupBy::Endpoint => "endpoint",
    }
}

fn series_json(computed_at_ms: i64, request: &Request, series: &Series) -> Value {
    let q = &request.series;
    json!({
        "object": "stats_series",
        "computed_at_ms": computed_at_ms,
        "group_by": group_by_name(q.stats.group_by),
        "interval_ms": q.interval_ms,
        "window": {"since_ms": q.stats.since_ms, "seconds": request.window_ms / 1000},
        "now_ms": q.now_ms,
        "truncated": series.truncated,
        "covered_since_ms": series.covered_since_ms,
        "omitted_groups": series.omitted_groups,
        "data": series.groups.iter().map(|g| json!({
            "group": g.group,
            "buckets": g.buckets.iter().map(bucket_json).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn bucket_json(b: &Bucket) -> Value {
    json!({
        "start_ms": b.start_ms,
        "requests": b.requests,
        "attempts": b.attempts,
        "failures": b.failures,
        "failure_rate": b.failure_rate,
        "fallbacks": b.fallbacks,
        "fallback_rate": b.fallback_rate,
        "latency_ms": {"p50": b.p50_ms, "p90": b.p90_ms, "p99": b.p99_ms},
    })
}
