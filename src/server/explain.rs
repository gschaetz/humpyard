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

use super::{
    AppState, GatewayError, RequestInfo, Snapshot, authenticate, check_budget, plan_route,
};
use crate::config::CREDENTIAL_HEADERS;
use crate::ledger::{GroupBy, GroupStats, StatsQuery};
use crate::policy::{BudgetState, RequestMeta};
use crate::select::{Facts, Features};

/// What the caller says about the hypothetical request. Only `model` is required.
// A flat JSON request body: each flag is an independent fact the caller may state.
#[allow(clippy::struct_excessive_bools)]
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
    /// Estimated prompt size, for rules on `prompt_tokens`.
    prompt_tokens: Option<u64>,
    /// Whether the request defines tools.
    #[serde(default)]
    tools: bool,
    /// Whether the request carries images.
    #[serde(default)]
    images: bool,
}

pub(super) async fn explain_route(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let format = WireFormat::OpenAiChat;
    let snap = state.snapshot();
    let caller = match authenticate(&snap, &headers).await {
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
    let recent = recent_by_target(&state, caller.as_ref()).await;
    axum::Json(explain(&state, &snap, caller.as_ref(), &request, &recent)).into_response()
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

/// The distinct classes of `targets`, in the order they first appear. Targets without a class
/// are left out: the report says only what is known.
/// One entry per selector rule: whether it matched and, if not, why.
fn rules_json(trace: &[crate::select::RuleTrace<'_>]) -> Vec<Value> {
    trace
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
        .collect()
}

/// The planned route: its targets (each with its health), fallback kinds, classes, what a strict
/// route excluded, and what health skipped or would fail open on (ADR 0017, ADR 0018).
fn plan_report(
    report: &mut Value,
    snap: &Snapshot,
    route: &str,
    planned: &super::Planned,
    recent: &HashMap<String, GroupStats>,
    latency: &crate::latency::Latency,
) {
    let mut models: Vec<ModelId> = planned.plan.models.models_for(&Category::Any).to_vec();
    report["outcome"] = json!("ok");
    report["fallback_on"] = fallback_on_report(&planned.plan.fallback);
    report["fallback_includes_unavailable"] = json!(
        planned
            .plan
            .fallback
            .allows(crate::config::FallbackClass::Unavailable)
    );
    // A `fastest` route tries its targets by speed: show the order a request would use now (without
    // exploration, which is a per-request draw) and how often it explores.
    let tail: Vec<String> = snap
        .routes
        .get(route)
        .map(crate::routing::Route::tail_names)
        .unwrap_or_default();
    if let Some(route) = snap.routes.get(route) {
        // The tail is never ranked with the members: it follows them (ADR 0024).
        let names: Vec<String> = models
            .iter()
            .map(ToString::to_string)
            .filter(|n| !tail.contains(n))
            .collect();
        let quality = |target: &str| snap.config.target_quality.get(target).copied();
        if let Some(ranked) = route.speed_order(latency, &names, &quality) {
            models = ranked
                .order
                .iter()
                .chain(
                    tail.iter()
                        .filter(|t| models.iter().any(|m| m.as_str() == t.as_str())),
                )
                .map(|n| ModelId::from(n.as_str()))
                .collect();
            report["ordering"] = json!({
                "type": "fastest",
                "order": ranked.order,
                "explore": route.explore(),
                "quality_weight": route.quality_weight(),
                "scores": ranked.scores.iter().map(|s| json!({
                    "target": s.target,
                    "observed_ms": s.observed_ms,
                    "speed": s.speed,
                    "quality": s.quality,
                    "score": s.score,
                })).collect::<Vec<_>>(),
            });
        }
    }
    if let Some(spec) = snap.config.routes.get(route).and_then(|s| s.tail()) {
        report["tail"] = json!({
            "targets": spec.targets,
            "on": spec
                .guard()
                .classes()
                .unwrap_or_default()
                .iter()
                .map(|c| c.as_str())
                .collect::<Vec<_>>(),
        });
    }
    // A pool route: what it was asked to contain and who qualified (ADR 0021).
    if let Some(pool) = snap.config.pools.get(route) {
        report["pool"] = json!({
            "strength": pool.strength,
            "min_context_window": pool.min_context_window,
            "min_quality": pool.min_quality,
            "members": pool.members,
            "left_out": pool
                .left_out
                .iter()
                .map(|l| json!({"target": l.target, "reason": l.reason}))
                .collect::<Vec<_>>(),
        });
    }
    report["targets"] = Value::Array(
        models
            .iter()
            .map(|name| target_report(snap, name, recent, latency))
            .collect(),
    );
    report["incapable_targets"] = json!(planned.incapable);
    report["health_skipped_targets"] = json!(planned.cold_skipped);
    report["health_fail_open"] = json!(planned.fail_open);
    class_report(report, snap, route, &models);
}

/// Which failures hand a request to the next target: `"all"` or the listed kinds.
fn fallback_on_report(fallback: &crate::config::FallbackOn) -> Value {
    match fallback.classes() {
        None => json!("all"),
        Some(classes) => json!(classes.iter().map(|c| c.as_str()).collect::<Vec<_>>()),
    }
}

/// The route's classes: which its eligible targets span, whether it is strict, and what a strict
/// route's `allowed_classes` excluded (ADR 0017).
fn class_report(report: &mut Value, snap: &Snapshot, route: &str, targets: &[ModelId]) {
    let classes = distinct_classes(snap, targets);
    report["fallback_can_cross_classes"] = json!(classes.len() > 1);
    report["classes"] = json!(classes);
    report["allowed_classes"] = json!(route_spec_allowed(snap, route));
    report["excluded_targets"] = Value::Array(
        super::excluded_targets(snap, route)
            .into_iter()
            .map(|target| {
                json!({
                    "target": target,
                    "class": snap.config.target_classes.get(&target),
                    "reason": "its class is not in the route's allowed_classes",
                })
            })
            .collect(),
    );
}

/// The route's `allowed_classes`, or `null` when it is not strict.
fn route_spec_allowed<'a>(snap: &'a Snapshot, route: &str) -> Option<&'a [String]> {
    snap.config
        .routes
        .get(route)
        .and_then(crate::config::RouteSpec::allowed_classes)
}

fn distinct_classes(snap: &Snapshot, targets: &[ModelId]) -> Vec<String> {
    let mut distinct: Vec<String> = Vec::new();
    for name in targets {
        if let Some(class) = snap.config.target_classes.get(name.as_str())
            && !distinct.contains(class)
        {
            distinct.push(class.clone());
        }
    }
    distinct
}

/// How each target has behaved for the caller over the last day, when a ledger exists.
async fn recent_by_target(
    state: &AppState,
    caller: Option<&crate::auth::KeyRecord>,
) -> HashMap<String, GroupStats> {
    let Some(ledger) = state.accounting.ledger() else {
        return HashMap::new();
    };
    // Decisions queued a moment ago should count.
    ledger.flush().await;
    let query = StatsQuery {
        key: caller.map(|k| k.id.clone()),
        since_ms: state.clock.now_ms() - super::stats::DEFAULT_WINDOW_MS,
        group_by: GroupBy::Target,
        route: None,
        target: None,
        row_cap: 50_000,
    };
    ledger
        .stats(&query)
        .await
        .map(|s| s.groups.into_iter().map(|g| (g.group.clone(), g)).collect())
        .unwrap_or_default()
}

fn explain(
    state: &AppState,
    snap: &Snapshot,
    caller: Option<&crate::auth::KeyRecord>,
    request: &Hypothetical,
    recent: &HashMap<String, GroupStats>,
) -> Value {
    let header_facts = request.header_facts();
    let info = request.request_info();
    // The caller's `prompt_tokens` stands for both estimates: it is the best number the caller has.
    let features = Features {
        prompt_tokens: request.prompt_tokens,
        prompt_tokens_low: request.prompt_tokens,
        tools: request.tools,
        images: request.images,
    };
    let facts = Facts {
        model: &request.model,
        key: caller.map(|k| k.id.as_str()),
        headers: &header_facts,
        agent: request.agent.as_deref(),
        task: request.task.as_deref(),
        subagent: request.subagent,
        stream: request.stream,
        features,
    };
    let permitted = |route: &str| caller.is_none_or(|key| key.may_use(route));
    let trace = snap.selectors.trace(&facts, permitted);
    let rules = rules_json(&trace);

    // Resolve the route exactly as a real request does.
    let (route_name, rule, source) = match super::select_route(
        snap,
        caller,
        &request.model,
        request.stream,
        facts.features,
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
                "outcome": error.outcome(),
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
            report["outcome"] = json!(error.outcome());
            report["message"] = json!(error.to_string());
            return report;
        }
    };
    report["budget"] = json!(budget.map(budget_name));
    let route = snap.routes.get(&route_name);
    match route.map(|r| {
        plan_route(
            snap,
            r,
            &route_name,
            &super::PlanInput {
                caller,
                budget,
                info: &info,
                features: &features,
                latency: None,
            },
        )
    }) {
        Some(Ok(planned)) => plan_report(
            &mut report,
            snap,
            &route_name,
            &planned,
            recent,
            state.accounting.latency(),
        ),
        Some(Err(failure)) => {
            report["outcome"] = json!(failure.error.outcome());
            report["message"] = json!(failure.error.to_string());
            report["incapable_targets"] = json!(failure.incapable);
        }
        None => {
            report["outcome"] = json!("unknown_model");
        }
    }
    report
}

fn target_report(
    snap: &Snapshot,
    name: &ModelId,
    recent: &HashMap<String, GroupStats>,
    latency: &crate::latency::Latency,
) -> Value {
    let figure = latency.figure(name.as_str());
    let endpoints: Vec<Value> = snap
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
                        "cooldown_source": e.snapshot.cooldown_source.map(crate::health::CooldownSource::as_str),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let recent = recent.get(name.as_str()).map(|g| {
        json!({
            "window_hours": 24,
            "attempts": g.attempts,
            "failure_rate": g.failure_rate(),
            "p50_ms": g.p50_ms,
        })
    });
    json!({
        "target": name.to_string(),
        "class": snap.config.target_classes.get(name.as_str()),
        "cold": super::is_cold(snap, name.as_str()),
        "generated": snap.config.generated_targets.contains(name.as_str()),
        "quality": snap.config.target_quality.get(name.as_str()),
        "current_latency_ms": figure.map(|f| f.ms),
        "latency_samples": figure.map(|f| f.samples),
        "endpoints": endpoints,
        "recent": recent,
    })
}

fn budget_name(state: BudgetState) -> &'static str {
    match state {
        BudgetState::Healthy => "healthy",
        BudgetState::Restricted => "restricted",
        BudgetState::Exhausted => "exhausted",
    }
}
