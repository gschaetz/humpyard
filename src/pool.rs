//! Provider pool: one client per target, each walking the target's ordered endpoints.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use http::{HeaderName, HeaderValue, StatusCode};
use switchyard_llm_client::{Backend, HttpBackendConfig, ModelConfig, TranslatingLlmClient};
use switchyard_protocol::{LlmClientError, ModelId, Request, Response, RoutedLlmClient};

use crate::clock::Clock;
use crate::config::{Config, Endpoint, Price, Provider};
use crate::health::{Admission, Breaker, Snapshot, Transition};

/// Response header naming the provider endpoint that served a request.
pub const PROVIDER_HEADER: &str = "x-humpyard-provider";

struct EndpointClient {
    provider: String,
    model: String,
    price: Option<Price>,
    client: TranslatingLlmClient,
    breaker: Breaker,
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
        }
        _ => false,
    }
}

#[async_trait]
impl RoutedLlmClient for TargetClient {
    async fn call(&self, request: Request) -> Result<Response, LlmClientError> {
        self.call_detailed(request)
            .await
            .map(|(response, _)| response)
    }
}

impl TargetClient {
    /// The endpoints to try, in order: those not cooling down. When every endpoint is cooling
    /// down the target is still tried, soonest-to-reopen first, so health never denies service.
    fn walk(&self) -> Vec<(usize, ProbeSlot<'_>)> {
        let mut admitted = Vec::new();
        for (index, endpoint) in self.endpoints.iter().enumerate() {
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
            let mut all: Vec<usize> = (0..self.endpoints.len()).collect();
            all.sort_by_key(|&i| self.endpoints[i].breaker.cooldown_remaining_ms());
            admitted = all
                .into_iter()
                .map(|i| (i, ProbeSlot { breaker: None }))
                .collect();
        }
        admitted
    }

    /// Like `call`, also reporting which endpoint served the response.
    pub async fn call_detailed(
        &self,
        request: Request,
    ) -> Result<(Response, Served), LlmClientError> {
        let mut last_error = None;
        let mut walk = self.walk();
        let count = walk.len();
        for (position, (index, slot)) in walk.iter_mut().enumerate() {
            let endpoint = &self.endpoints[*index];
            let mut attempt = request.clone();
            attempt.llm_request.model = Some(endpoint.model.clone());
            match endpoint.client.call(attempt).await {
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
                        price: endpoint.price,
                    };
                    return Ok((response, served));
                }
                Err(error) if fails_over(&error) => {
                    slot.resolved();
                    endpoint.log(endpoint.breaker.failure());
                    if position + 1 >= count {
                        return Err(error);
                    }
                    tracing::warn!(
                        provider = %endpoint.provider,
                        model = %endpoint.model,
                        error = %error,
                        "endpoint failed; trying next"
                    );
                    last_error = Some(error);
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
    config: &Config,
    clock: &Arc<dyn Clock>,
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
        price: endpoint.price,
        client,
        breaker: Breaker::new(config.health, clock.clone()),
    })
}

type TargetEndpoint = EndpointClient;

/// Builds one client per configured target.
pub fn build(
    config: &Config,
    clock: &Arc<dyn Clock>,
) -> Result<HashMap<ModelId, Arc<TargetClient>>, String> {
    let mut targets = HashMap::new();
    for (id, endpoints) in &config.targets {
        let clients = endpoints
            .iter()
            .map(|endpoint| {
                endpoint_client(
                    &config.providers[&endpoint.provider],
                    endpoint,
                    config,
                    clock,
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("target `{id}`: {e}"))?;
        targets.insert(
            ModelId::from(id.as_str()),
            Arc::new(TargetClient { endpoints: clients }),
        );
    }
    Ok(targets)
}
