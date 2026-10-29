//! `GET /v1/decisions`: the routing decision log, newest first. A key reads only its own
//! decisions (an open gateway reads all) until an admin credential exists (ADR 0013).

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use switchyard_protocol::WireFormat;

use super::{AppState, GatewayError, authenticate};
use crate::ledger::{Decision, DecisionQuery};
use crate::signals::Signals;

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 500;

pub(super) async fn list_decisions(
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
    decisions_view(&state, &params, caller.map(|k| k.id)).await
}

/// The decision log for `key` (every key when `None`). The client endpoint passes the caller's own id; the
/// admin endpoint passes the `key` filter it was given.
pub(super) async fn decisions_view(
    state: &AppState,
    params: &HashMap<String, String>,
    key: Option<String>,
) -> Response {
    let format = WireFormat::OpenAiChat;
    let Some(ledger) = state.accounting.ledger() else {
        return GatewayError::ModelNotFound(
            "no ledger is configured, so decisions are not recorded".into(),
        )
        .into_response_for(format);
    };
    let query = match parse(params, key) {
        Ok(query) => query,
        Err(message) => return GatewayError::BadRequest(message).into_response_for(format),
    };
    // Decisions queued a moment ago should be visible to the request that follows them.
    ledger.flush().await;
    match ledger.decisions(&query).await {
        Ok(decisions) => {
            // The serving call's signals come from its usage row; a failed lookup only leaves them out.
            let ids: Vec<String> = decisions.iter().map(|d| d.request_id.clone()).collect();
            let signals = ledger.answer_signals(&ids).await.unwrap_or_default();
            axum::Json(json!({
                "object": "list",
                "data": decisions
                    .iter()
                    .map(|d| to_json(d, signals.get(&d.request_id).copied()))
                    .collect::<Vec<_>>(),
            }))
            .into_response()
        }
        Err(error) => GatewayError::Internal(error.to_string()).into_response_for(format),
    }
}

fn parse(params: &HashMap<String, String>, key: Option<String>) -> Result<DecisionQuery, String> {
    for name in params.keys() {
        if ![
            "limit",
            "since_ms",
            "route",
            "rule",
            "outcome",
            "request_id",
        ]
        .contains(&name.as_str())
        {
            return Err(format!("unknown filter `{name}`"));
        }
    }
    let number = |name: &str| -> Result<Option<i64>, String> {
        params
            .get(name)
            .map(|v| {
                v.parse::<i64>()
                    .map_err(|_| format!("`{name}` must be a whole number"))
            })
            .transpose()
    };
    let limit = number("limit")?.map_or(DEFAULT_LIMIT, |n| {
        u32::try_from(n.clamp(1, i64::from(MAX_LIMIT))).unwrap_or(DEFAULT_LIMIT)
    });
    Ok(DecisionQuery {
        key,
        route: params.get("route").cloned(),
        rule: params.get("rule").cloned(),
        outcome: params.get("outcome").cloned(),
        request_id: params.get("request_id").cloned(),
        since_ms: number("since_ms")?,
        limit,
    })
}

fn to_json(d: &Decision, signals: Option<Signals>) -> Value {
    let mut request = json!({
        "stream": d.stream,
        "prompt_tokens_estimate": d.prompt_tokens,
        "tools": d.tools,
        "images": d.images,
    });
    // Only when some tool call was repaired (ADR 0029); the count, never the arguments.
    if let Some(count) = d.repaired_tool_arguments {
        request["repaired_tool_arguments"] = json!(count);
    }
    let mut view = json!({
        "request_id": d.request_id,
        "ts_ms": d.ts_ms,
        "key": d.key_id,
        "session_id": d.session_id,
        "format": d.format,
        "requested_model": d.requested_model,
        "route": d.route,
        "rule": d.rule,
        "source": d.source,
        "skipped_rules": d.skipped_rules,
        "budget": d.budget,
        "eligible_targets": d.eligible_targets,
        "excluded_targets": d.excluded_targets,
        "cold_targets": d.cold_targets,
        "incapable_targets": d.incapable_targets,
        "overrides": d.overrides,
        "cost_ceiling": d.cost_ceiling,
        "ranking": d.ranking,
        "request": request,
        "attempts": d.attempts,
        "fallback_crossed_classes": d.crossed_classes(),
        "status": d.status,
        "outcome": d.outcome,
        "message": d.message,
        "served": {"target": d.served_target, "provider": d.served_provider},
        "latency_ms": d.latency_ms,
    });
    // The facts set on the call that served the request (ADR 0030); absent when none is set or they are unknown.
    let names = signals.map(Signals::names).unwrap_or_default();
    if !names.is_empty() {
        view["response_signals"] = json!(names);
    }
    view
}
