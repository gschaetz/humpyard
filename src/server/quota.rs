//! `GET /v1/quota`: the latest usage windows of every provider that names a quota source, with
//! the age of the data, or why it is unknown. Read-only; credentials never appear (ADR 0015).

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use switchyard_protocol::WireFormat;

use super::stats::{DEFAULT_WINDOW_MS, parse_window};
use super::{AppState, GatewayError, authenticate};
use crate::ledger::QuotaQuery;
use crate::quota::{View, Window, identity};

fn window_json(window: &Window) -> Value {
    json!({
        "name": window.name,
        "percent": window.percent,
        "status": window.status,
        "resets_at": window.resets_at,
        "clamped": window.clamped,
    })
}

pub(super) async fn get_quota(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let snap = state.snapshot();
    if let Err(error) = authenticate(&snap, &headers).await {
        return error.into_response_for(WireFormat::OpenAiChat);
    }
    quota_report(&state, &snap)
}

/// The latest usage windows of every provider that names a quota source; the client endpoint and the admin API
/// serve the same report.
pub(super) fn quota_report(state: &AppState, snap: &super::Snapshot) -> Response {
    let config = &snap.config;
    let now = state.clock.now_ms();
    let stale_ms = i64::try_from(config.quota_stale_secs.saturating_mul(1000)).unwrap_or(i64::MAX);
    let providers: serde_json::Map<String, Value> = config
        .providers
        .iter()
        .filter_map(|(name, provider)| {
            let source = provider.quota_source.as_deref()?;
            let view = state
                .quota
                .view(name, &identity(source, &provider.base_url), now, stale_ms);
            let body = match view {
                View::Fresh {
                    windows,
                    fetched_ms,
                    age_ms,
                    last_error,
                } => json!({
                    "source": source,
                    "state": "fresh",
                    "fetched_at_ms": fetched_ms,
                    "age_secs": age_ms / 1000,
                    "windows": windows.iter().map(window_json).collect::<Vec<_>>(),
                    "last_error": last_error,
                }),
                View::Unknown { reason } => json!({
                    "source": source,
                    "state": "unknown",
                    "reason": reason,
                }),
            };
            Some((name.clone(), body))
        })
        .collect();
    axum::Json(json!({
        "object": "quota",
        "poll_secs": config.quota_poll_secs,
        "stale_secs": config.quota_stale_secs,
        "providers": providers,
    }))
    .into_response()
}

/// Longest history window when the retention is "forever".
const UNBOUNDED_WINDOW_MS: i64 = 365 * 86_400_000;
const ROW_CAP: u32 = 20_000;

/// `GET /v1/quota/history`: the stored quota samples, oldest first, each with the notional cost
/// of the provider's calls since the previous sample of its window. This is the data for judging
/// what a provider's percent is a percent of; it is account-wide, so it can move with traffic
/// that did not come through this gateway.
pub(super) async fn get_history(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    let format = WireFormat::OpenAiChat;
    let snap = state.snapshot();
    if let Err(error) = authenticate(&snap, &headers).await {
        return error.into_response_for(format);
    }
    let Some(ledger) = state.accounting.ledger() else {
        return GatewayError::ModelNotFound(
            "no ledger is configured, so quota history is not available".into(),
        )
        .into_response_for(format);
    };
    for name in params.keys() {
        if !["window", "provider", "name"].contains(&name.as_str()) {
            return GatewayError::BadRequest(format!("unknown parameter `{name}`"))
                .into_response_for(format);
        }
    }
    let longest = match snap.config.decision_retention_days {
        0 => UNBOUNDED_WINDOW_MS,
        days => i64::try_from(days.saturating_mul(86_400_000)).unwrap_or(UNBOUNDED_WINDOW_MS),
    };
    let window_ms = match params
        .get("window")
        .map_or(Ok(DEFAULT_WINDOW_MS), |w| parse_window(w))
    {
        Ok(ms) => ms.min(longest),
        Err(message) => return GatewayError::BadRequest(message).into_response_for(format),
    };
    let now = state.clock.now_ms();
    let query = QuotaQuery {
        since_ms: now.saturating_sub(window_ms),
        provider: params.get("provider").cloned(),
        window: params.get("name").cloned(),
        row_cap: ROW_CAP,
    };
    ledger.flush().await;
    match ledger.quota_history(&query).await {
        Ok(points) => axum::Json(json!({
            "object": "quota_history",
            "window": {"since_ms": query.since_ms, "seconds": (now - query.since_ms) / 1000},
            "data": points,
        }))
        .into_response(),
        Err(error) => GatewayError::Internal(error.to_string()).into_response_for(format),
    }
}
