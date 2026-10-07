//! HTTP surface: inference endpoints in three client protocols, served through Switchyard's
//! `run` over the provider pool.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures::{Stream, StreamExt};
use serde_json::{Value, json};

use switchyard_llm_client::run;
use switchyard_protocol::{Category, LlmResponse, Metadata, ModelId, Request, WireFormat};
use switchyard_translation::{
    LlmStreamError, RawEventStream, decode_request, encode_aggregated_response_with_extensions,
    encode_stream_with_extensions, util::SWITCHYARD_METADATA_KEY,
};

use crate::auth::{ConfigKeyStore, KeyRecord, KeyStore, hash_key, presented_key};
use crate::budget::{BudgetPolicy, BudgetTracker};
use crate::clock::Clock;
use crate::clock::SystemClock;
use crate::config::Config;
use crate::config::OverBudget;
use crate::error::GatewayError;
use crate::ledger::Ledger;
use crate::metering::{Accounting, CALL_ID_HEADER, CallContext};
use crate::num::f64_from_u64;
use crate::policy::{
    All, AllowAll, BudgetState, KeyContext, PolicyContext, RequestMeta, RoutingPolicy,
};
use crate::pool::{self, PROVIDER_HEADER, TargetClient};
use crate::routing::Routes;

const TARGET_HEADER: &str = "x-humpyard-target";

pub struct AppState {
    config: Config,
    targets: HashMap<ModelId, Arc<TargetClient>>,
    accounting: Arc<Accounting>,
    tracker: Option<Arc<BudgetTracker>>,
    routes: Routes,
    policy: Arc<dyn RoutingPolicy>,
    keys: Arc<dyn KeyStore>,
}

pub async fn router(config: Config) -> Result<Router, String> {
    router_with_options(config, Options::default()).await
}

/// Like [`router`], with a custom policy deciding which targets are eligible per request. The
/// budget policy always applies in addition.
pub async fn router_with_policy(
    config: Config,
    policy: Arc<dyn RoutingPolicy>,
) -> Result<Router, String> {
    router_with_options(
        config,
        Options {
            policy: Some(policy),
            ..Options::default()
        },
    )
    .await
}

/// Embedding hooks: an extra routing policy and a clock (tests control budget periods with it).
#[derive(Default)]
pub struct Options {
    pub policy: Option<Arc<dyn RoutingPolicy>>,
    pub clock: Option<Arc<dyn Clock>>,
}

pub async fn router_with_options(config: Config, options: Options) -> Result<Router, String> {
    let clock: Arc<dyn Clock> = options.clock.unwrap_or_else(|| Arc::new(SystemClock));
    let targets = pool::build(&config)?;
    let ledger = match &config.ledger {
        Some(path) => Some(Ledger::open(path).await.map_err(|e| e.to_string())?),
        None => None,
    };
    let tracker = (!config.keys.is_empty())
        .then(|| Arc::new(BudgetTracker::new(clock.clone(), &config.budget)));
    if let (Some(tracker), Some(ledger)) = (&tracker, &ledger) {
        tracker.hydrate(ledger).await.map_err(|e| e.to_string())?;
    }
    let accounting = Arc::new(Accounting::new(ledger, clock, tracker.clone()));
    let policy: Arc<dyn RoutingPolicy> = Arc::new(All(vec![
        options.policy.unwrap_or_else(|| Arc::new(AllowAll)),
        Arc::new(BudgetPolicy::new(&config)),
    ]));
    let routes = Routes::build(&config)?;
    let keys: Arc<dyn KeyStore> = Arc::new(ConfigKeyStore::new(&config.keys));
    let state = Arc::new(AppState {
        config,
        targets,
        accounting,
        tracker,
        routes,
        policy,
        keys,
    });
    Ok(Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .route("/v1/models", get(list_models))
        .route("/v1/key/info", get(key_info))
        .route(
            "/v1/chat/completions",
            post(|s, h, b| infer(s, WireFormat::OpenAiChat, h, b)),
        )
        .route(
            "/v1/responses",
            post(|s, h, b| infer(s, WireFormat::OpenAiResponses, h, b)),
        )
        .route(
            "/v1/messages",
            post(|s, h, b| infer(s, WireFormat::AnthropicMessages, h, b)),
        )
        .with_state(state))
}

async fn list_models(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let caller = match authenticate(&state, &headers).await {
        Ok(caller) => caller,
        Err(error) => return error.into_response_for(WireFormat::OpenAiChat),
    };
    let data: Vec<Value> = state
        .config
        .model_names()
        .into_iter()
        .filter(|id| caller.as_ref().is_none_or(|key| key.may_use(id)))
        .map(|id| json!({"id": id, "object": "model", "created": 0, "owned_by": "humpyard"}))
        .collect();
    axum::Json(json!({"object": "list", "data": data})).into_response()
}

/// The calling key's id, limits, spend and budget state.
async fn key_info(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    let format = WireFormat::OpenAiChat;
    let caller = match authenticate(&state, &headers).await {
        Ok(Some(caller)) => caller,
        Ok(None) => {
            return GatewayError::ModelNotFound("key info needs a key: the gateway is open".into())
                .into_response_for(format);
        }
        Err(error) => return error.into_response_for(format),
    };
    let Some(tracker) = &state.tracker else {
        return GatewayError::Internal("budget tracking is not running".into())
            .into_response_for(format);
    };
    let status = tracker.status(&caller.id, &caller.limits);
    let usd = |micro: u64| f64_from_u64(micro) / 1_000_000.0;
    let remaining = |limit: Option<f64>, spent: f64| limit.map(|l| (l - spent).max(0.0));
    axum::Json(json!({
        "id": caller.id,
        "state": match status.state {
            BudgetState::Healthy => "healthy",
            BudgetState::Restricted => "restricted",
            BudgetState::Exhausted => "exhausted",
        },
        "binding_limit": status.binding_limit,
        "limits": {
            "daily_usd": status.limits.daily_usd,
            "monthly_usd": status.limits.monthly_usd,
            "daily_tokens": status.limits.daily_tokens,
            "monthly_tokens": status.limits.monthly_tokens,
        },
        "spend": {
            "daily_usd": usd(status.day.micro_usd),
            "monthly_usd": usd(status.month.micro_usd),
            "daily_tokens": status.day.tokens,
            "monthly_tokens": status.month.tokens,
        },
        "remaining": {
            "daily_usd": remaining(status.limits.daily_usd, usd(status.day.micro_usd)),
            "monthly_usd": remaining(status.limits.monthly_usd, usd(status.month.micro_usd)),
            "daily_tokens": remaining(status.limits.daily_tokens.map(f64_from_u64), f64_from_u64(status.day.tokens)),
            "monthly_tokens": remaining(status.limits.monthly_tokens.map(f64_from_u64), f64_from_u64(status.month.tokens)),
        },
    }))
    .into_response()
}

/// Identifies the caller. `None` means the gateway is open (no keys configured).
async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<KeyRecord>, GatewayError> {
    if !state.keys.enforces_auth() {
        return Ok(None);
    }
    let key = presented_key(headers).ok_or_else(|| {
        GatewayError::Unauthorized(
            "missing API key: send `Authorization: Bearer <key>` or `x-api-key`".into(),
        )
    })?;
    match state.keys.lookup(&hash_key(key)).await {
        Some(record) => Ok(Some(record)),
        None => Err(GatewayError::Unauthorized("invalid API key".into())),
    }
}

async fn infer(
    State(state): State<Arc<AppState>>,
    format: WireFormat,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let started = std::time::Instant::now();
    let caller = match authenticate(&state, &headers).await {
        Ok(caller) => caller,
        Err(error) => {
            tracing::warn!(%format, status = error.status().as_u16(), error = %error, "request rejected");
            return error.into_response_for(format);
        }
    };
    let key_id = caller.as_ref().map_or("-", |k| k.id.as_str()).to_string();
    match handle(&state, format, &headers, &body, caller.as_ref()).await {
        Ok((target, response)) => {
            tracing::info!(
                %format,
                key = key_id,
                target,
                provider = response
                    .headers()
                    .get(PROVIDER_HEADER)
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
                status = response.status().as_u16(),
                elapsed_ms = started.elapsed().as_millis(),
                "request"
            );
            response
        }
        Err(error) => {
            tracing::warn!(%format, key = key_id, status = error.status().as_u16(), error = %error, "request failed");
            error.into_response_for(format)
        }
    }
}

// Split into named stages in `harden-engineering` task 4.3, which removes this allowance.
#[allow(clippy::too_many_lines)]
async fn handle(
    state: &AppState,
    format: WireFormat,
    headers: &HeaderMap,
    raw: &[u8],
    caller: Option<&KeyRecord>,
) -> Result<(String, Response), GatewayError> {
    let mut body: Value = serde_json::from_slice(raw)
        .map_err(|e| GatewayError::BadRequest(format!("invalid JSON body: {e}")))?;
    // Only trusted translation hops may supply preservation state; strip any from clients.
    if let Some(metadata) = body.get_mut("metadata").and_then(Value::as_object_mut) {
        metadata.remove(SWITCHYARD_METADATA_KEY);
    }
    let llm_request = decode_request(format, &body)
        .map_err(|e| GatewayError::BadRequest(format!("cannot translate request: {e}")))?;
    let model = llm_request
        .model
        .clone()
        .filter(|m| !m.trim().is_empty())
        .ok_or_else(|| GatewayError::BadRequest("`model` is required".into()))?;
    if format == WireFormat::OpenAiResponses
        && body
            .get("previous_response_id")
            .is_some_and(|v| !v.is_null())
    {
        return Err(GatewayError::BadRequest(
            "`previous_response_id` is not supported: stateful responses are unavailable".into(),
        ));
    }
    if let Some(key) = caller
        && !key.may_use(&model)
    {
        return Err(GatewayError::Forbidden(format!(
            "this key may not use `{model}`"
        )));
    }
    let route = state
        .routes
        .get(&model)
        .ok_or_else(|| GatewayError::ModelNotFound(model.clone()))?;

    let extensions = llm_request.extensions.clone();
    // `wire_format` stays unset: setting it pins the backend to the client's protocol, but every
    // endpoint speaks OpenAI Chat and the IR is translated for it.
    let metadata = Metadata::from_headers(headers);
    let session_id = metadata.session_id.clone().filter(|s| !s.is_empty());
    let request_meta = RequestMeta {
        agent_id: metadata.agent_id.clone(),
        parent_agent_id: metadata.parent_agent_id.clone(),
        is_subagent: metadata.is_subagent,
        task_id: metadata.task_id.clone(),
    };
    let budget = caller
        .zip(state.tracker.as_ref())
        .map(|(key, tracker)| (key, tracker.status(&key.id, &key.limits)));
    if let Some((key, status)) = &budget
        && status.state == BudgetState::Exhausted
        && key.over_budget == OverBudget::Block
    {
        return Err(GatewayError::BudgetExceeded(format!(
            "budget exhausted: key `{}` reached its {}",
            key.id,
            status.binding_limit.unwrap_or("limit")
        )));
    }
    let budget_state = budget.as_ref().map(|(_, status)| status.state);
    let context = PolicyContext {
        route: &model,
        session_id: session_id.as_deref(),
        metadata: &request_meta,
        key: caller.map(|key| KeyContext {
            id: &key.id,
            budget: budget_state.unwrap_or(BudgetState::Healthy),
        }),
    };
    let plan = route
        .plan(&model, |target| state.policy.is_eligible(&context, target))
        .ok_or_else(|| {
            if budget_state == Some(BudgetState::Exhausted) {
                GatewayError::BudgetExceeded(format!(
                    "budget exhausted and route `{model}` has no free target to continue on"
                ))
            } else {
                GatewayError::Unavailable(format!(
                    "no eligible target for route `{model}`: all are excluded by routing policy"
                ))
            }
        })?;
    let names: Vec<ModelId> = plan.models.models_for(&Category::Any).to_vec();
    let models = Arc::new(plan.models);
    let ctx = CallContext::new(
        state.accounting.clone(),
        caller.map(|k| k.id.clone()),
        session_id,
        model.clone(),
    );
    let clients = ctx.router(&state.targets, &names);
    let request = Request {
        llm_request,
        raw_request: Some(body),
        metadata: Some(metadata),
    };

    let (selected, mut response) = match run(plan.algorithm, clients, request, models, None).await {
        Ok(done) => done,
        Err(error) => {
            ctx.finish_err();
            return Err(error.into());
        }
    };
    let answer_id = response
        .upstream_headers
        .remove(CALL_ID_HEADER)
        .and_then(|v| v.to_str().ok()?.parse::<u64>().ok());
    let stream_template = ctx.finish_ok(answer_id);

    let served = response.served_model().unwrap_or(&selected).to_string();
    let upstream_headers = std::mem::take(&mut response.upstream_headers);
    let mut http_response = match response.llm_response {
        LlmResponse::Agg(agg) => {
            let body = encode_aggregated_response_with_extensions(
                &agg,
                format,
                Some(&served),
                &extensions,
            )
            .map_err(|e| GatewayError::Internal(format!("cannot encode response: {e}")))?;
            axum::Json(body).into_response()
        }
        LlmResponse::Stream(stream) => {
            let stream = match stream_template {
                Some(template) => ctx.tap(stream, template),
                None => stream,
            };
            let events =
                encode_stream_with_extensions(stream, format, Some(served.clone()), &extensions)
                    .map_err(|e| GatewayError::Internal(e.to_string()))?;
            frame_stream(events, format).into_response()
        }
    };
    set_header(&mut http_response, TARGET_HEADER, &served);
    if let Some(provider) = upstream_headers
        .get(PROVIDER_HEADER)
        .and_then(|v| v.to_str().ok())
    {
        set_header(&mut http_response, PROVIDER_HEADER, provider);
    }
    Ok((served, http_response))
}

fn set_header(response: &mut Response, name: &'static str, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        response
            .headers_mut()
            .insert(HeaderName::from_static(name), value);
    }
}

type SseStream = std::pin::Pin<Box<dyn Stream<Item = Result<Event, Infallible>> + Send>>;

/// Frames translated events as SSE for the client's protocol. Dropping the returned body drops
/// the upstream stream, which cancels the upstream request.
fn frame_stream(stream: RawEventStream, format: WireFormat) -> Sse<SseStream> {
    let framed = async_stream::stream! {
        let mut stream = stream;
        let mut failed = false;
        while let Some(item) = stream.next().await {
            let event = match item {
                Ok(value) => frame_event(format, &value),
                Err(LlmStreamError::Upstream(value)) => {
                    failed = true;
                    frame_event(format, &value)
                }
                Err(LlmStreamError::Client(error)) => {
                    tracing::warn!("stream iteration failed");
                    failed = true;
                    error_event(format, &error.to_string())
                }
            };
            yield Ok(event);
            if failed {
                break;
            }
        }
        // `[DONE]` marks a successful OpenAI Chat stream; it must not follow a failed turn.
        if !failed && format == WireFormat::OpenAiChat {
            yield Ok(Event::default().data("[DONE]"));
        }
    };
    Sse::new(Box::pin(framed) as SseStream)
}

/// Anthropic and Responses clients expect a named SSE event matching the payload's `type`.
fn frame_event(format: WireFormat, value: &Value) -> Event {
    let data = value.to_string();
    match format {
        WireFormat::OpenAiChat => Event::default().data(data),
        WireFormat::AnthropicMessages | WireFormat::OpenAiResponses => {
            let name = value["type"].as_str().unwrap_or("message");
            Event::default().event(name).data(data)
        }
    }
}

fn error_event(format: WireFormat, message: &str) -> Event {
    let body = GatewayError::UpstreamUnreachable(message.to_string()).body(format);
    match format {
        WireFormat::OpenAiChat => Event::default().data(body.to_string()),
        _ => Event::default().event("error").data(body.to_string()),
    }
}
