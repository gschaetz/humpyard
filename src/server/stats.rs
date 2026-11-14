//! `GET /v1/stats`: how routes, targets and endpoints have actually performed, from the decision
//! log and usage rows. Read-only, and under the same per-key read model as `GET /v1/decisions`.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use switchyard_protocol::WireFormat;

use super::heavy::{Busy, Surface};
use super::{AppState, GatewayError, authenticate};
use crate::config::Price;
use crate::ledger::{GroupBy, GroupStats, StatsQuery, TokenTotals};
use crate::pricing::{BaselineRefusal, baseline_price, linear_cost_micro_usd};

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
    stats_view(
        &state,
        &snap,
        &params,
        caller.map(|k| k.id),
        Surface::Client,
    )
    .await
}

/// Statistics for `key` (every key when `None`): the client endpoint passes the caller's own id, the admin
/// endpoint the `key` filter it was given.
pub(super) async fn stats_view(
    state: &AppState,
    snap: &super::Snapshot,
    params: &HashMap<String, String>,
    key: Option<String>,
    surface: Surface,
) -> Response {
    // Heavy: at most one at a time per surface, and identical reads share one result (ADR 0036).
    state
        .heavy
        .run(
            surface,
            "stats",
            key.as_deref(),
            params,
            stats_answer(state, snap, params, key.clone(), surface),
        )
        .await
}

/// The statistics answer itself, computed once for however many identical reads wait on it.
async fn stats_answer(
    state: &AppState,
    snap: &super::Snapshot,
    params: &HashMap<String, String>,
    key: Option<String>,
    surface: Surface,
) -> Result<Response, Busy> {
    let format = WireFormat::OpenAiChat;
    let Some(ledger) = state.accounting.ledger() else {
        return Ok(GatewayError::ModelNotFound(
            "no ledger is configured, so statistics are not available".into(),
        )
        .into_response_for(format));
    };
    let longest = longest_window(snap);
    if params.get("group_by").is_some_and(|g| g == "session") {
        return super::sessions::sessions_view(state, key, params, longest).await;
    }
    let query = match parse(params, key, state.clock.now_ms(), longest, surface) {
        Ok(query) => query,
        Err(message) => {
            return Ok(GatewayError::BadRequest(message).into_response_for(format));
        }
    };
    // A baseline that cannot be used is refused before anything is read.
    let baseline = match params
        .get("baseline")
        .map(|name| resolve_baseline(snap, name))
    {
        None => None,
        Some(Ok(found)) => Some(found),
        Some(Err(message)) => {
            return Ok(GatewayError::BadRequest(message).into_response_for(format));
        }
    };
    ledger.flush().await;
    match ledger.stats(&query).await {
        Ok(summary) => {
            let mut answer = json!({
                "object": "stats",
                "computed_at_ms": state.clock.now_ms(),
                "group_by": params.get("group_by").map_or("route", String::as_str),
                "window": {"since_ms": query.since_ms, "seconds": (state.clock.now_ms() - query.since_ms) / 1000},
                "truncated": summary.truncated,
                "data": summary.groups.iter().map(|g| {
                    let mut row = group_json(g);
                    if params.get("group_by").map(String::as_str) == Some("target") {
                        add_live_latency(&mut row, state, &g.group);
                    }
                    if let Some(baseline) = &baseline {
                        row["vs_baseline"] = baseline.group(g).json();
                    }
                    row
                }).collect::<Vec<_>>(),
            });
            if let Some(baseline) = &baseline {
                baseline.describe(&summary.groups, &mut answer);
            }
            Ok(axum::Json(answer).into_response())
        }
        Err(error) if error.is_busy() => Err(Busy),
        Err(error) => Ok(GatewayError::Internal(error.to_string()).into_response_for(format)),
    }
}

/// A baseline endpoint: what the same calls would have cost at its prices (`add-baseline-cost`).
struct Baseline {
    endpoint: String,
    price: Price,
}

/// What a baseline says about one group, or about the whole answer, in micro-USD.
#[derive(Default)]
struct Compared {
    baseline_micro: f64,
    actual_micro: u64,
    priced: u64,
    left_out: u64,
}

impl Compared {
    fn saved_micro(&self) -> f64 {
        self.baseline_micro - crate::num::f64_from_u64(self.actual_micro)
    }

    fn json(&self) -> Value {
        let usd = |micro: f64| micro / 1_000_000.0;
        json!({
            "baseline_cost_usd": usd(self.baseline_micro),
            "cost_usd": usd(crate::num::f64_from_u64(self.actual_micro)),
            "saved_usd": usd(self.saved_micro()),
            // Null when the baseline cost is nothing, so a ratio is never made of zero.
            "saved_ratio": (self.baseline_micro > 0.0).then(|| self.saved_micro() / self.baseline_micro),
            "calls_priced": self.priced,
            "calls_left_out": self.left_out,
        })
    }
}

const BASELINE_BASIS: &str = "An estimate, not a measurement: the tokens these calls used, priced at this endpoint's prices from the configuration. It says nothing about whether this endpoint would have answered as well, and another model counts tokens differently. Calls whose provider reported no usage are in neither cost.";

impl Baseline {
    /// The comparison for one group: its calls with known usage priced at the baseline, and what they cost.
    fn group(&self, g: &GroupStats) -> Compared {
        let t = &g.tokens;
        Compared {
            baseline_micro: linear_cost_micro_usd(
                &self.price,
                t.input.saturating_add(t.cache_creation),
                t.cached_input,
                t.output,
            ),
            actual_micro: g.cost_micro_usd,
            priced: g.usage_known,
            left_out: g.answered.saturating_sub(g.usage_known),
        }
    }

    /// Adds the baseline and the summary of every group to the answer.
    fn describe(&self, groups: &[GroupStats], answer: &mut Value) {
        let mut sum = Compared::default();
        for g in groups {
            let c = self.group(g);
            sum.baseline_micro += c.baseline_micro;
            sum.actual_micro = sum.actual_micro.saturating_add(c.actual_micro);
            sum.priced = sum.priced.saturating_add(c.priced);
            sum.left_out = sum.left_out.saturating_add(c.left_out);
        }
        let p = &self.price;
        answer["baseline"] = json!({
            "endpoint": self.endpoint,
            "input_per_million_usd": p.input,
            "cached_input_per_million_usd": p.cached_input.unwrap_or(p.input),
            "output_per_million_usd": p.output,
            "basis": BASELINE_BASIS,
        });
        answer["summary"] = sum.json();
    }
}

/// The baseline named `name` (`provider/model`), or the message of the 400 that refuses it.
fn resolve_baseline(snap: &super::Snapshot, name: &str) -> Result<Baseline, String> {
    let endpoints = snap
        .config
        .targets
        .values()
        .flatten()
        .map(|e| (e.provider.as_str(), e.model.as_str(), e.price.as_ref()));
    match baseline_price(endpoints, name) {
        Ok(price) => Ok(Baseline {
            endpoint: name.to_string(),
            price: price.clone(),
        }),
        Err(BaselineRefusal::Unknown) => Err(format!(
            "`baseline` must be an endpoint of the configuration, as `provider/model`; `{name}` is not one"
        )),
        Err(BaselineRefusal::NoPrice) => Err(format!(
            "`baseline` `{name}` has no price in the configuration, so it cannot be priced"
        )),
        Err(BaselineRefusal::Tiered) => Err(format!(
            "`baseline` `{name}` has a tiered price; tiered baselines are not supported, because a call's cost then depends on its prompt size"
        )),
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
    surface: Surface,
) -> Result<StatsQuery, String> {
    for name in params.keys() {
        // `baseline` shows the prices of configured endpoints, which a client key must not learn, so only the
        // administrative surface takes it (`add-baseline-cost`).
        let known = ["window", "group_by", "route", "target"].contains(&name.as_str())
            || (name == "baseline" && surface == Surface::Admin);
        if !known {
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
        // A client key sees only its own calls, so grouping them by actor or owner could add nothing.
        Some("actor") if surface == Surface::Admin => GroupBy::Actor,
        Some("owner") if surface == Surface::Admin => GroupBy::Owner,
        Some(other) => {
            return Err(if surface == Surface::Admin {
                format!("`group_by` must be route, target, endpoint, actor or owner, not `{other}`")
            } else {
                format!("`group_by` must be route, target or endpoint, not `{other}`")
            });
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
