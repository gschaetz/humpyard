//! `POST /v1/route/explain`: a dry run of the request path up to the upstream call. It answers
//! "given this key and these facts, which rule applies, which route and targets would be used, and
//! why not the others?" using the same selection, budget and policy code real requests use, so it
//! cannot drift from them. It never calls a provider and never writes the ledger.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use switchyard_protocol::{Category, Metadata, ModelId, WireFormat};

use super::{AppState, GatewayError, RequestInfo, authenticate, check_budget, plan_route};
use crate::config::CREDENTIAL_HEADERS;
use crate::policy::{BudgetState, RequestMeta};
use crate::select::Facts;

/// What the caller says about the hypothetical request. Only `model` is required.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Hypothetical {
    model: String,
    #[serde(default)]
    stream: bool,
    /// Request headers the client would send (credential headers are ignored).
    #[serde(default)]
    headers: HashMap<String, String>,
    agent: Option<String>,
    task: Option<String>,
    #[serde(default)]
    subagent: bool,
}

pub(super) async fn explain_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let format = WireFormat::OpenAiChat;
    let caller = match authenticate(&state, &headers).await {
        Ok(caller) => caller,
        Err(error) => return error.into_response_for(format),
    };
    let request: Hypothetical = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(error) => {
            return GatewayError::BadRequest(format!(
                "explain needs a JSON body like {{\"model\": \"...\", \"headers\": {{...}}}}: {error}"
            ))
            .into_response_for(format);
        }
    };
    axum::Json(explain(&state, caller.as_ref(), &request)).into_response()
}

impl Hypothetical {
    /// Headers as selector rules see them: lower-cased, credential headers dropped.
    fn header_facts(&self) -> HashMap<String, String> {
        self.headers
            .iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
            .filter(|(k, _)| !CREDENTIAL_HEADERS.contains(&k.as_str()))
            .collect()
    }

    fn request_info(&self) -> RequestInfo {
        RequestInfo {
            metadata: Metadata::default(),
            session_id: None,
            meta: RequestMeta {
                agent_id: self.agent.clone(),
                parent_agent_id: None,
                is_subagent: self.subagent,
                task_id: self.task.clone(),
            },
        }
    }
}

fn explain(
    state: &AppState,
    caller: Option<&crate::auth::KeyRecord>,
    request: &Hypothetical,
) -> Value {
    let header_facts = request.header_facts();
    let info = request.request_info();
    let facts = Facts {
        model: &request.model,
        key: caller.map(|k| k.id.as_str()),
        headers: &header_facts,
        agent: request.agent.as_deref(),
        task: request.task.as_deref(),
        subagent: request.subagent,
        stream: request.stream,
    };
    let permitted = |route: &str| caller.is_none_or(|key| key.may_use(route));
    let trace = state.selectors.trace(&facts, permitted);
    let rules: Vec<Value> = trace
        .iter()
        .map(|t| {
            json!({
                "rule": t.rule,
                "route": t.route,
                "matched": t.mismatches.is_empty(),
                "key_may_use_route": t.permitted,
                "mismatches": t.mismatches,
            })
        })
        .collect();

    // Resolve the route exactly as a real request does.
    let (route_name, rule, source) = match super::select_route(
        state,
        caller,
        &request.model,
        request.stream,
        &header_facts,
        &info,
    ) {
        Ok(selection) => {
            let source = if selection.rule == "default" {
                "requested_model"
            } else {
                "selector"
            };
            (selection.name, selection.rule, source)
        }
        Err(error) => {
            return json!({
                "key": caller.map(|k| &k.id),
                "requested_model": request.model,
                "outcome": outcome_name(&error),
                "message": error.to_string(),
                "rules": rules,
            });
        }
    };

    let mut report = json!({
        "key": caller.map(|k| &k.id),
        "requested_model": request.model,
        "selected": {"route": route_name, "rule": rule, "source": source},
        "rules": rules,
    });
    let budget = match check_budget(state, caller) {
        Ok(budget) => budget,
        Err(error) => {
            report["outcome"] = json!(outcome_name(&error));
            report["message"] = json!(error.to_string());
            return report;
        }
    };
    report["budget"] = json!(budget.map(budget_name));
    let route = state.routes.get(&route_name);
    match route.map(|r| plan_route(state, r, &route_name, caller, budget, &info)) {
        Some(Ok(plan)) => {
            report["outcome"] = json!("ok");
            report["fallback_on"] = match plan.fallback.classes() {
                None => json!("all"),
                Some(classes) => json!(classes.iter().map(|c| c.as_str()).collect::<Vec<_>>()),
            };
            let targets: Vec<Value> = plan
                .models
                .models_for(&Category::Any)
                .iter()
                .map(|name| target_report(state, name))
                .collect();
            report["targets"] = Value::Array(targets);
        }
        Some(Err(error)) => {
            report["outcome"] = json!(outcome_name(&error));
            report["message"] = json!(error.to_string());
        }
        None => {
            report["outcome"] = json!("unknown_model");
        }
    }
    report
}

fn target_report(state: &AppState, name: &ModelId) -> Value {
    let endpoints: Vec<Value> = state
        .targets
        .get(name)
        .map(|client| {
            client
                .health()
                .into_iter()
                .map(|e| {
                    json!({
                        "provider": e.provider,
                        "model": e.model,
                        "state": e.snapshot.state.as_str(),
                        "cooldown_remaining_ms": e.snapshot.cooldown_remaining_ms,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({"target": name.to_string(), "endpoints": endpoints})
}

fn budget_name(state: BudgetState) -> &'static str {
    match state {
        BudgetState::Healthy => "healthy",
        BudgetState::Restricted => "restricted",
        BudgetState::Exhausted => "exhausted",
    }
}

/// A stable machine-readable name for why a request would not be served.
fn outcome_name(error: &GatewayError) -> &'static str {
    match error {
        GatewayError::Forbidden(_) => "forbidden",
        GatewayError::ModelNotFound(_) => "unknown_model",
        GatewayError::BudgetExceeded(_) => "budget_exhausted",
        GatewayError::Unavailable(_) => "no_eligible_target",
        _ => "error",
    }
}
