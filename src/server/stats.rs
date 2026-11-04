//! `GET /v1/stats`: how routes, targets and endpoints have actually performed, from the decision
//! log and usage rows. Read-only, and under the same per-key read model as `GET /v1/decisions`.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use switchyard_protocol::WireFormat;

use super::{AppState, GatewayError, authenticate};
use crate::ledger::{GroupBy, GroupStats, StatsQuery, TokenTotals};

pub(super) const DEFAULT_WINDOW_MS: i64 = 24 * 3_600_000;
/// The window when the decision retention is "forever".
const UNBOUNDED_WINDOW_MS: i64 = 365 * 86_400_000;
pub(super) const ROW_CAP: u32 = 50_000;

pub(super) async fn get_stats(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let format = WireFormat::OpenAiChat;
    let snap = state.snapshot();
    let caller = match authenticate(&snap, &headers).await {
        Ok(caller) => caller,
        Err(error) => return error.into_response_for(format),
    };
    stats_view(&state, &snap, &params, caller.map(|k| k.id)).await
}

/// Statistics for `key` (every key when `None`): the client endpoint passes the caller's own id, the admin
/// endpoint the `key` filter it was given.
pub(super) async fn stats_view(
    state: &AppState,
    snap: &super::Snapshot,
    params: &HashMap<String, String>,
    key: Option<String>,
) -> Response {
    let format = WireFormat::OpenAiChat;
    let Some(ledger) = state.accounting.ledger() else {
        return GatewayError::ModelNotFound(
            "no ledger is configured, so statistics are not available".into(),
        )
        .into_response_for(format);
    };
    let longest = longest_window(snap);
    if params.get("group_by").is_some_and(|g| g == "session") {
        return super::sessions::sessions_view(state, key, params, longest).await;
    }
    let query = match parse(params, key, state.clock.now_ms(), longest) {
        Ok(query) => query,
        Err(message) => return GatewayError::BadRequest(message).into_response_for(format),
    };
    ledger.flush().await;
    match ledger.stats(&query).await {
        Ok(summary) => axum::Json(json!({
            "object": "stats",
            "group_by": params.get("group_by").map_or("route", String::as_str),
            "window": {"since_ms": query.since_ms, "seconds": (state.clock.now_ms() - query.since_ms) / 1000},
            "truncated": summary.truncated,
            "data": summary.groups.iter().map(|g| {
                let mut row = group_json(g);
                if params.get("group_by").map(String::as_str) == Some("target") {
                    add_live_latency(&mut row, state, &g.group);
                }
                row
            }).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(error) => GatewayError::Internal(error.to_string()).into_response_for(format),
    }
}

/// The longest window a caller may ask for: the decision retention (or a year when it is forever).
pub(super) fn longest_window(snap: &super::Snapshot) -> i64 {
    match snap.config.decision_retention_days {
        0 => UNBOUNDED_WINDOW_MS,
        days => i64::try_from(days.saturating_mul(86_400_000)).unwrap_or(UNBOUNDED_WINDOW_MS),
    }
}

/// `30s`, `15m`, `24h` or `7d` as milliseconds.
pub(super) fn parse_window(text: &str) -> Result<i64, String> {
    let bad = || format!("`window` must look like 30m, 24h or 7d, not `{text}`");
    let (digits, unit) = text.split_at(text.len().saturating_sub(1));
    let n: i64 = digits.parse().map_err(|_| bad())?;
    let unit_ms = match unit {
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return Err(bad()),
    };
    if n <= 0 {
        return Err(bad());
    }
    Ok(n.saturating_mul(unit_ms))
}

fn parse(
    params: &HashMap<String, String>,
    key: Option<String>,
    now_ms: i64,
    longest_ms: i64,
) -> Result<StatsQuery, String> {
    for name in params.keys() {
        if !["window", "group_by", "route", "target"].contains(&name.as_str()) {
            return Err(format!("unknown parameter `{name}`"));
        }
    }
    let window = params
        .get("window")
        .map_or(Ok(DEFAULT_WINDOW_MS), |w| parse_window(w))?
        .min(longest_ms);
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
    Ok(StatsQuery {
        key,
        since_ms: now_ms.saturating_sub(window),
        group_by,
        route: params.get("route").cloned(),
        target: params.get("target").cloned(),
        row_cap: ROW_CAP,
    })
}

fn rate(part: u64, whole: u64) -> Option<f64> {
    (whole > 0).then(|| crate::num::f64_from_u64(part) / crate::num::f64_from_u64(whole))
}

/// The token split of a group: fresh input, cached input, cache creation and output.
pub(super) fn tokens_json(t: &TokenTotals) -> Value {
    json!({
        "input": t.input,
        "cached_input": t.cached_input,
        "cache_creation": t.cache_creation,
        "output": t.output,
    })
}

/// A target's current smoothed time to answer, as `fastest` routes see it (in memory, so it can
/// differ from the percentiles over the window): `null` until the target has answered.
fn add_live_latency(row: &mut Value, state: &AppState, target: &str) {
    let figure = state.accounting.latency().figure(target);
    row["current_latency_ms"] = json!(figure.map(|f| f.ms));
    row["latency_samples"] = json!(figure.map(|f| f.samples));
}

fn group_json(g: &GroupStats) -> Value {
    json!({
        "group": g.group,
        "requests": g.requests,
        "attempts": g.attempts,
        "outcomes": g.outcomes,
        "failures": g.failures,
        "failure_rate": g.failure_rate(),
        "fallbacks": g.fallbacks,
        "fallback_rate": rate(g.fallbacks, g.requests),
        "latency_ms": {"p50": g.p50_ms, "p90": g.p90_ms, "p99": g.p99_ms},
        "answered": g.answered,
        "avg_cost_usd": g.avg_cost_micro_usd.map(|c| c / 1_000_000.0),
        "avg_tokens": g.avg_tokens,
        "tokens": tokens_json(&g.tokens),
        "cache_hit_ratio": g.tokens.cache_hit_ratio(),
        "response_signals": {
            "observed": g.signals.observed,
            "empty_output": g.signals.empty_output,
            "truncated_output": g.signals.truncated_output,
            "bad_tool_call": g.signals.bad_tool_call,
            "cut_stream": g.signals.cut_stream,
        },
    })
}
