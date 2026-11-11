//! Session analytics endpoints: `GET /v1/stats?group_by=session` and `GET /v1/stats/switches`. Read-only
//! views over the usage rows, under the same per-key read model as `GET /v1/stats` (ADR 0013). They
//! exist to measure what switching models in the middle of a session costs before anything pins a
//! session to a target. A session id is an opaque client string, returned only to the key that sent it.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use switchyard_protocol::WireFormat;

use super::heavy::{Busy, Surface};
use super::stats::{DEFAULT_WINDOW_MS, ROW_CAP, longest_window, parse_window, tokens_json};
use super::{AppState, GatewayError, Snapshot, authenticate};
use crate::ledger::{SessionQuery, SessionScan, SessionStats, Switch};
use crate::num::{f64_from_u64, u64_from_f64_rounded};

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;
/// The endpoint pairs listed in the switch report.
const TOP_PAIRS: usize = 10;

const NOTE: &str = "Descriptive, not causal: sessions that switch are probably longer and harder than ones that \
    stay. warm_tokens_abandoned is an approximation: what the call before each switch left cached, which \
    the new endpoint pays the fresh input price for.";

#[derive(Clone, Copy)]
enum Order {
    Cost,
    Requests,
    Switches,
    Recent,
}

fn bad(message: impl Into<String>) -> Response {
    GatewayError::BadRequest(message.into()).into_response_for(WireFormat::OpenAiChat)
}

fn window_ms(params: &HashMap<String, String>, longest_ms: i64) -> Result<i64, String> {
    Ok(params
        .get("window")
        .map_or(Ok(DEFAULT_WINDOW_MS), |w| parse_window(w))?
        .min(longest_ms))
}

fn ratio(part: u64, whole: u64) -> Option<f64> {
    (whole > 0).then(|| f64_from_u64(part) / f64_from_u64(whole))
}

fn usd(micro: u64) -> f64 {
    f64_from_u64(micro) / 1_000_000.0
}

/// `group_by=session`: the caller's sessions in the window, costliest first by default.
pub(super) async fn sessions_view(
    state: &AppState,
    key: Option<String>,
    params: &HashMap<String, String>,
    longest_ms: i64,
) -> Result<Response, Busy> {
    for name in params.keys() {
        if !["window", "group_by", "limit", "order_by"].contains(&name.as_str()) {
            return Ok(bad(format!(
                "unknown parameter `{name}` for group_by=session"
            )));
        }
    }
    let window = match window_ms(params, longest_ms) {
        Ok(window) => window,
        Err(message) => return Ok(bad(message)),
    };
    let limit = match params.get("limit").map(|l| l.parse::<usize>()) {
        None => DEFAULT_LIMIT,
        Some(Ok(n)) if (1..=MAX_LIMIT).contains(&n) => n,
        Some(_) => return Ok(bad(format!("`limit` must be 1 to {MAX_LIMIT}"))),
    };
    let order = match params.get("order_by").map(String::as_str) {
        None | Some("cost") => Order::Cost,
        Some("requests") => Order::Requests,
        Some("switches") => Order::Switches,
        Some("recent") => Order::Recent,
        Some(other) => {
            return Ok(bad(format!(
                "`order_by` must be cost, requests, switches or recent, not `{other}`"
            )));
        }
    };
    let Some(ledger) = state.accounting.ledger() else {
        return Ok(no_ledger());
    };
    ledger.flush().await;
    let now = state.clock.now_ms();
    let since_ms = now.saturating_sub(window);
    let scan = match ledger
        .session_scan(&SessionQuery {
            key,
            since_ms,
            row_cap: ROW_CAP,
        })
        .await
    {
        Ok(scan) => scan,
        Err(error) if error.is_busy() => return Err(Busy),
        Err(error) => {
            return Ok(
                GatewayError::Internal(error.to_string()).into_response_for(WireFormat::OpenAiChat)
            );
        }
    };
    let sessions_total = scan.sessions.len();
    let mut sessions: Vec<&SessionStats> = scan.sessions.iter().collect();
    sessions.sort_by(|a, b| {
        let key = |s: &SessionStats| match order {
            Order::Cost => s.cost_micro_usd,
            Order::Requests => s.calls,
            Order::Switches => s.switches,
            Order::Recent => u64::try_from(s.ended_ms).unwrap_or(0),
        };
        key(b).cmp(&key(a)).then_with(|| a.session.cmp(&b.session))
    });
    Ok(axum::Json(json!({
        "object": "stats",
        "computed_at_ms": now,
        "group_by": "session",
        "order_by": params.get("order_by").map_or("cost", String::as_str),
        "window": {"since_ms": since_ms, "seconds": window / 1000},
        "truncated": scan.truncated,
        "sessions_total": sessions_total,
        "answered_without_session": scan.answered.saturating_sub(scan.answered_with_session),
        "data": sessions.into_iter().take(limit).map(session_json).collect::<Vec<_>>(),
    }))
    .into_response())
}

fn session_json(s: &SessionStats) -> Value {
    json!({
        "group": s.session,
        "requests": s.calls,
        "cost_micro_usd": s.cost_micro_usd,
        "cost_usd": usd(s.cost_micro_usd),
        "tokens": tokens_json(&s.tokens),
        "cache_hit_ratio": s.tokens.cache_hit_ratio(),
        "endpoints": s.endpoints,
        "switches": s.switches,
        "started_ms": s.started_ms,
        "ended_ms": s.ended_ms,
    })
}

fn no_ledger() -> Response {
    GatewayError::ModelNotFound("no ledger is configured, so statistics are not available".into())
        .into_response_for(WireFormat::OpenAiChat)
}

/// `GET /v1/stats/switches`: what changing endpoint in the middle of a session abandoned.
pub(super) async fn get_switches(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let snap = state.snapshot();
    let caller = match authenticate(&snap, &headers).await {
        Ok(caller) => caller,
        Err(error) => return error.into_response_for(WireFormat::OpenAiChat),
    };
    for name in params.keys() {
        if name != "window" {
            return bad(format!("unknown parameter `{name}`"));
        }
    }
    let window = match window_ms(&params, longest_window(&snap)) {
        Ok(window) => window,
        Err(message) => return bad(message),
    };
    let key = caller.map(|k| k.id);
    // Heavy: at most one at a time per surface, and identical reads share one result (ADR 0036).
    state
        .heavy
        .run(
            Surface::Client,
            "switches",
            key.as_deref(),
            &params,
            switches_answer(&state, &snap, key.clone(), window),
        )
        .await
}

async fn switches_answer(
    state: &AppState,
    snap: &Snapshot,
    key: Option<String>,
    window: i64,
) -> Result<Response, Busy> {
    let Some(ledger) = state.accounting.ledger() else {
        return Ok(no_ledger());
    };
    ledger.flush().await;
    let now = state.clock.now_ms();
    let since_ms = now.saturating_sub(window);
    let scan = match ledger
        .session_scan(&SessionQuery {
            key,
            since_ms,
            row_cap: ROW_CAP,
        })
        .await
    {
        Ok(scan) => scan,
        Err(error) if error.is_busy() => return Err(Busy),
        Err(error) => {
            return Ok(
                GatewayError::Internal(error.to_string()).into_response_for(WireFormat::OpenAiChat)
            );
        }
    };
    let mut body = switches_json(snap, &scan, since_ms, window);
    body["computed_at_ms"] = json!(now);
    Ok(axum::Json(body).into_response())
}

/// The fresh and cached input prices (USD per million tokens) of each `provider/model` endpoint that
/// has a price; the first target that names an endpoint wins.
fn input_prices(snap: &Snapshot) -> HashMap<String, (f64, f64)> {
    let mut prices = HashMap::new();
    for endpoint in snap.config.targets.values().flatten() {
        if let Some(price) = &endpoint.price {
            prices
                .entry(format!("{}/{}", endpoint.provider, endpoint.model))
                .or_insert((price.input, price.cached_input.unwrap_or(price.input)));
        }
    }
    prices
}

/// What the abandoned tokens cost at the destination's prices, in micro-USD (per-million USD is
/// numerically micro-USD per token), or `None` when the destination has no price.
fn extra_cost(prices: &HashMap<String, (f64, f64)>, switch: &Switch) -> Option<u64> {
    let (fresh, cached) = prices.get(&switch.to)?;
    let per_token = (fresh - cached).max(0.0);
    Some(u64_from_f64_rounded(
        f64_from_u64(switch.warm_tokens) * per_token,
    ))
}

fn cohort(sessions: &[&SessionStats]) -> Value {
    let n = u64::try_from(sessions.len()).unwrap_or(u64::MAX);
    let calls: u64 = sessions.iter().map(|s| s.calls).sum();
    let cost: u64 = sessions.iter().map(|s| s.cost_micro_usd).sum();
    json!({
        "sessions": n,
        "avg_calls": ratio(calls, n),
        "avg_cost_usd": ratio(cost, n).map(|c| c / 1_000_000.0),
    })
}

fn switches_json(snap: &Snapshot, scan: &SessionScan, since_ms: i64, window_ms: i64) -> Value {
    let endpoint_prices = input_prices(snap);
    let sessions_total = u64::try_from(scan.sessions.len()).unwrap_or(u64::MAX);
    let switched: Vec<&SessionStats> = scan.sessions.iter().filter(|s| s.switches > 0).collect();
    let stayed: Vec<&SessionStats> = scan.sessions.iter().filter(|s| s.switches == 0).collect();
    let warm_total: u64 = scan.switches.iter().map(|s| s.warm_tokens).sum();
    let costs: Vec<u64> = scan
        .switches
        .iter()
        .filter_map(|s| extra_cost(&endpoint_prices, s))
        .collect();
    let extra_total: Option<u64> = (!costs.is_empty()).then(|| costs.iter().sum());

    // The pairs that switch most, with the tokens they abandoned.
    let mut pairs: BTreeMap<(String, String), (u64, u64)> = BTreeMap::new();
    for switch in &scan.switches {
        let entry = pairs
            .entry((switch.from.clone(), switch.to.clone()))
            .or_default();
        entry.0 += 1;
        entry.1 = entry.1.saturating_add(switch.warm_tokens);
    }
    let mut pairs: Vec<_> = pairs.into_iter().collect();
    pairs.sort_by(|a, b| b.1.0.cmp(&a.1.0).then_with(|| a.0.cmp(&b.0)));

    json!({
        "object": "session_switches",
        "window": {"since_ms": since_ms, "seconds": window_ms / 1000},
        "truncated": scan.truncated,
        "coverage": {
            "answered": scan.answered,
            "with_session_id": scan.answered_with_session,
            "ratio": ratio(scan.answered_with_session, scan.answered),
            "note": "Everything below covers only the answered calls that carried a session id.",
        },
        "sessions": {"total": sessions_total, "switched": switched.len()},
        "switches": {
            "count": scan.switches.len(),
            "warm_tokens_abandoned": warm_total,
            "priced": costs.len(),
            "unpriced": scan.switches.len() - costs.len(),
            "estimated_extra_cost_usd": extra_total.map(usd),
        },
        "spend": {
            "total_usd": usd(scan.total_cost_micro_usd),
            "extra_share": extra_total.and_then(|extra| ratio(extra, scan.total_cost_micro_usd)),
        },
        "by_pair": pairs.into_iter().take(TOP_PAIRS).map(|((from, to), (count, warm))| json!({
            "from": from, "to": to, "count": count, "warm_tokens_abandoned": warm,
        })).collect::<Vec<_>>(),
        "stayed": cohort(&stayed),
        "switched": cohort(&switched),
        "note": NOTE,
    })
}
