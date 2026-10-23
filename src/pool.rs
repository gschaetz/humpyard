//! Provider pool: one client per target, each walking the target's ordered endpoints.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use http::{HeaderName, HeaderValue, StatusCode};
use switchyard_llm_client::{Backend, HttpBackendConfig, ModelConfig, TranslatingLlmClient};
use switchyard_protocol::{
    ContentBlock, LlmClientError, ModelId, Request, Response, Role, RoutedLlmClient,
};

use crate::clock::Clock;
use crate::config::{Config, Endpoint, HealthConfig, Price, Provider};
use crate::error::{TIMEOUT_MARKER, fallback_class, is_forbidden, is_too_large, is_unavailable};
use crate::health::{Admission, Breaker, Snapshot, Transition};
use crate::overrides::Drains;

/// Response header naming the provider endpoint that served a request.
pub const PROVIDER_HEADER: &str = "x-humpyard-provider";

struct EndpointClient {
    provider: String,
    model: String,
    price: Option<Price>,
    client: TranslatingLlmClient,
    /// Shared with the next snapshot on a reload, so health survives edits that do not touch it.
    breaker: Arc<Breaker>,
    /// Part of the endpoint's identity: a different URL is a different endpoint.
    base_url: String,
}

/// One endpoint's health, for reporting.
#[derive(Clone, Debug)]
pub struct EndpointHealth {
    pub provider: String,
    pub model: String,
    pub snapshot: Snapshot,
}

/// Holds a recovery probe's slot; unless the call reached a verdict (or no call was made), the
/// slot is released on drop so a cancelled request cannot leave the endpoint waiting for a probe
/// that never reports.
struct ProbeSlot<'a> {
    breaker: Option<&'a Breaker>,
}

impl ProbeSlot<'_> {
    fn resolved(&mut self) {
        self.breaker = None;
    }
}

impl Drop for ProbeSlot<'_> {
    fn drop(&mut self) {
        if let Some(breaker) = self.breaker {
            breaker.release_probe();
        }
    }
}

/// What happened on one endpoint, reported to the caller as the walk proceeds (decision log).
#[derive(Clone, Debug)]
pub struct EndpointAttempt {
    pub provider: String,
    pub model: String,
    /// `ok`, the failure class (`rate_limit`, `timeout`, ...), or `error` for other failures.
    pub outcome: String,
    pub ms: u64,
    /// The retry time the endpoint stated (clamped), when this failure stated one and the endpoint was
    /// cooled for it.
    pub retry_after_ms: Option<u64>,
}

/// Which endpoint served a call, so usage can be attributed and priced.
#[derive(Clone, Debug)]
pub struct Served {
    pub provider: String,
    pub model: String,
    pub price: Option<Price>,
}

/// Serves one target by trying its endpoints in order.
pub struct TargetClient {
    endpoints: Vec<EndpointClient>,
    /// Providers a runtime override has drained; their endpoints are never tried (`add-runtime-overrides`).
    drains: Arc<dyn Drains>,
}

/// Failures that another endpoint of the same target may not share. Other client errors (bad
/// request, auth) would fail identically elsewhere, so they stop the walk.
fn fails_over(error: &LlmClientError) -> bool {
    match error {
        LlmClientError::Transport { .. } | LlmClientError::Timeout { .. } => true,
        LlmClientError::UpstreamHttp { status, .. } => {
            *status == StatusCode::TOO_MANY_REQUESTS
                || *status == StatusCode::REQUEST_TIMEOUT
                || status.is_server_error()
                // The model is gone here or the key is rejected: another endpoint of the target may
                // serve it, and this one counts as failing (ADR 0023).
                || is_unavailable(error)
        }
        _ => false,
    }
}

/// The retry time a 429 or 503 states, clamped to what `[health]` allows. Other statuses and errors
/// that state nothing readable return `None` and are handled by the ordinary failure count.
fn stated_retry_time(error: &LlmClientError, health: HealthConfig) -> Option<Duration> {
    match error {
        LlmClientError::UpstreamHttp { status, body }
            if *status == StatusCode::TOO_MANY_REQUESTS
                || *status == StatusCode::SERVICE_UNAVAILABLE =>
        {
            crate::retry_after::from_body(body)
                .map(|wait| crate::retry_after::clamp(wait, health.max_retry_after_secs))
        }
        _ => None,
    }
}

/// A timeout that exhausted a target is re-labelled as a gateway timeout (HTTP 504) so the route
/// falls back to its next target like it does for any 5xx; see `error::TIMEOUT_MARKER`.
fn for_target_fallback(error: LlmClientError) -> LlmClientError {
    match error {
        LlmClientError::Timeout { .. } => LlmClientError::UpstreamHttp {
            status: StatusCode::GATEWAY_TIMEOUT,
            body: TIMEOUT_MARKER.to_string(),
        },
        other => other,
    }
}

/// Adapts a request, whatever protocol it arrived in, for chat-completions providers:
///
/// - A reasoning item replayed from the Responses API (Codex does this each turn) can carry the
///   whole Responses item as a "detail"; chat providers reject its array-valued `summary`. Such
///   entries are dropped (the reasoning text itself is kept); genuine chat-style details, for
///   example `reasoning.encrypted`, are left untouched.
/// - The `developer` role (Codex injects it mid-conversation) is rejected by many providers, so it
///   is sent as `system`.
fn adapt_for_chat_providers(request: &mut Request) {
    // Responses developer messages are hoisted into `instructions` by the decoder.
    for instruction in &mut request.llm_request.instructions {
        if instruction.role == Role::Developer {
            instruction.role = Role::System;
        }
    }
    for message in &mut request.llm_request.messages {
        if message.role == Role::Developer {
            message.role = Role::System;
        }
        for block in &mut message.content {
            if let ContentBlock::Reasoning { details, .. } = block {
                details.retain(|d| d.get("type").and_then(|t| t.as_str()) != Some("reasoning"));
            }
        }
    }
}

#[async_trait]
impl RoutedLlmClient for TargetClient {
    async fn call(&self, request: Request) -> Result<Response, LlmClientError> {
        self.call_detailed(request, &mut |_| {})
            .await
            .map(|(response, _)| response)
    }
}

impl TargetClient {
    /// The endpoints to try, in order: those not cooling down. When every endpoint is cooling
    /// down the target is still tried, soonest-to-reopen first, so health never denies service.
    ///
    /// An endpoint on a drained provider is left out first, so the rule above never brings it back: a drain is
    /// an operator's instruction, health is only a guess.
    fn walk(&self) -> Vec<(usize, ProbeSlot<'_>)> {
        let usable: Vec<usize> = (0..self.endpoints.len())
            .filter(|&i| !self.drains.drained(&self.endpoints[i].provider))
            .collect();
        let mut admitted = Vec::new();
        for &index in &usable {
            let endpoint = &self.endpoints[index];
            match endpoint.breaker.admit() {
                Admission::Allowed => admitted.push((index, ProbeSlot { breaker: None })),
                Admission::Probe => admitted.push((
                    index,
                    ProbeSlot {
                        breaker: Some(&endpoint.breaker),
                    },
                )),
                Admission::Skip => {}
            }
        }
        if admitted.is_empty() {
            let mut all = usable;
            all.sort_by_key(|&i| self.endpoints[i].breaker.cooldown_remaining_ms());
            admitted = all
                .into_iter()
                .map(|i| (i, ProbeSlot { breaker: None }))
                .collect();
        }
        admitted
    }

    /// Like `call`, also reporting which endpoint served the response and, through `on_attempt`,
    /// every endpoint attempt as it ends.
    pub async fn call_detailed(
        &self,
        request: Request,
        on_attempt: &mut (dyn FnMut(EndpointAttempt) + Send),
    ) -> Result<(Response, Served), LlmClientError> {
        let mut request = request;
        adapt_for_chat_providers(&mut request);
        let mut last_error = None;
        let mut walk = self.walk();
        let count = walk.len();
        if count == 0 {
            // Planning leaves out a target whose endpoints are all drained, so this is a drain that began
            // while the request was in flight.
            return Err(LlmClientError::General(
                "every endpoint of this target is drained by a runtime override".into(),
            ));
        }
        for (position, (index, slot)) in walk.iter_mut().enumerate() {
            let endpoint = &self.endpoints[*index];
            let mut attempt = request.clone();
            attempt.llm_request.model = Some(endpoint.model.clone());
            let started = std::time::Instant::now();
            let result = endpoint.client.call(attempt).await;
            // A rate-limited or unavailable endpoint that says when it will serve again is cooled for
            // exactly that long (ADR 0018, amended); the breaker records it only for failures that
            // fail over.
            let wait_hint = result
                .as_ref()
                .err()
                .filter(|error| fails_over(error))
                .and_then(|error| stated_retry_time(error, endpoint.breaker.config()));
            on_attempt(EndpointAttempt {
                provider: endpoint.provider.clone(),
                model: endpoint.model.clone(),
                outcome: match &result {
                    Ok(_) => "ok".to_string(),
                    Err(error) => fallback_class(error)
                        .map_or("error", crate::config::FallbackClass::as_str)
                        .to_string(),
                },
                ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                retry_after_ms: wait_hint.map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX)),
            });
            match result {
                Ok(mut response) => {
                    slot.resolved();
                    endpoint.log(endpoint.breaker.success());
                    if let Ok(value) = HeaderValue::from_str(&endpoint.provider) {
                        response
                            .upstream_headers
                            .insert(HeaderName::from_static(PROVIDER_HEADER), value);
                    }
                    let served = Served {
                        provider: endpoint.provider.clone(),
                        model: endpoint.model.clone(),
                        price: endpoint.price.clone(),
                    };
                    return Ok((response, served));
                }
                Err(error) if fails_over(&error) => {
                    slot.resolved();
                    endpoint.log(match wait_hint {
                        Some(wait) => endpoint.breaker.failure_for(wait),
                        None => endpoint.breaker.failure(),
                    });
                    if position + 1 >= count {
                        return Err(for_target_fallback(error));
                    }
                    tracing::warn!(
                        provider = %endpoint.provider,
                        model = %endpoint.model,
                        error = %error,
                        "endpoint failed; trying next"
                    );
                    last_error = Some(error);
                }
                Err(error) if is_too_large(&error) => {
                    // A 413 says this request is too large for this endpoint, not that the endpoint is
                    // unwell: its health is left alone (the probe slot is released on drop) and another
                    // endpoint of the target may take the request (ADR 0023).
                    if position + 1 >= count {
                        return Err(error);
                    }
                    tracing::warn!(
                        provider = %endpoint.provider,
                        model = %endpoint.model,
                        "request too large for this endpoint; trying next"
                    );
                    last_error = Some(error);
                }
                Err(error) if is_forbidden(&error) => {
                    // A 403 describes this endpoint (the key may not use the model), so it counts against
                    // its health, but it does not move on to the next endpoint (ADR 0023).
                    slot.resolved();
                    endpoint.log(endpoint.breaker.failure());
                    return Err(error);
                }
                Err(error) => return Err(error),
            }
        }
        // Unreachable with a non-empty endpoint list; kept as an error rather than a panic.
        Err(last_error.unwrap_or_else(|| LlmClientError::General("target has no endpoints".into())))
    }

    /// Every endpoint's current health, in configured order.
    pub fn health(&self) -> Vec<EndpointHealth> {
        self.endpoints
            .iter()
            .map(|e| EndpointHealth {
                provider: e.provider.clone(),
                model: e.model.clone(),
                snapshot: e.breaker.snapshot(),
            })
            .collect()
    }
}

impl EndpointClient {
    fn log(&self, transition: Option<Transition>) {
        match transition {
            Some(Transition::Opened { cooldown_ms }) => tracing::warn!(
                provider = %self.provider,
                model = %self.model,
                cooldown_ms,
                "endpoint is failing; skipping it for the cooldown"
            ),
            Some(Transition::Recovered) => tracing::info!(
                provider = %self.provider,
                model = %self.model,
                "endpoint recovered"
            ),
            None => {}
        }
    }
}

fn endpoint_client(
    provider: &Provider,
    endpoint: &Endpoint,
    breaker: Arc<Breaker>,
) -> Result<TargetEndpoint, String> {
    let backend = Backend::OpenAiChat(HttpBackendConfig {
        base_url: provider.base_url.clone(),
        api_key: Some(provider.api_key.clone()),
        forward_auth: false,
        extra_headers: provider.headers.clone(),
        extra_body: std::collections::BTreeMap::default(),
        reasoning_effort: None,
        max_retries: provider.max_retries,
        timeout: Some(Duration::from_secs(provider.timeout_secs)),
    });
    let client = TranslatingLlmClient::new(&[ModelConfig::new(
        ModelId::from(endpoint.model.as_str()),
        backend,
        None,
    )])
    .map_err(|e| e.to_string())?;
    Ok(TargetEndpoint {
        provider: endpoint.provider.clone(),
        model: endpoint.model.clone(),
        price: endpoint.price.clone(),
        client,
        breaker,
        base_url: provider.base_url.clone(),
    })
}

type TargetEndpoint = EndpointClient;

impl TargetClient {
    /// The breaker of the endpoint with this identity, when this target has one and it was built
    /// with the same health settings, so a reload can keep what is known about it.
    fn reusable_breaker(
        &self,
        provider: &str,
        model: &str,
        base_url: &str,
        health: HealthConfig,
    ) -> Option<Arc<Breaker>> {
        self.endpoints
            .iter()
            .find(|e| e.provider == provider && e.model == model && e.base_url == base_url)
            .map(|e| e.breaker.clone())
            .filter(|b| b.config() == health)
    }
}

// The map type is this crate's own and always uses the default hasher.
#[allow(clippy::implicit_hasher)]
/// Builds one client per configured target. With `previous` (the targets of the config being
/// replaced, on a reload) the health state of every endpoint that did not change is carried over.
pub fn build(
    config: &Config,
    clock: &Arc<dyn Clock>,
    drains: &Arc<dyn Drains>,
    previous: Option<&HashMap<ModelId, Arc<TargetClient>>>,
) -> Result<HashMap<ModelId, Arc<TargetClient>>, String> {
    let mut targets = HashMap::new();
    for (id, endpoints) in &config.targets {
        let key = ModelId::from(id.as_str());
        let old = previous.and_then(|p| p.get(&key));
        let clients = endpoints
            .iter()
            .map(|endpoint| {
                let provider = &config.providers[&endpoint.provider];
                let breaker = old
                    .and_then(|t| {
                        t.reusable_breaker(
                            &endpoint.provider,
                            &endpoint.model,
                            &provider.base_url,
                            config.health,
                        )
                    })
                    .unwrap_or_else(|| Arc::new(Breaker::new(config.health, clock.clone())));
                endpoint_client(provider, endpoint, breaker)
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("target `{id}`: {e}"))?;
        targets.insert(
            key,
            Arc::new(TargetClient {
                endpoints: clients,
                drains: drains.clone(),
            }),
        );
    }
    Ok(targets)
}
