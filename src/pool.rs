//! Provider pool: one client per target, each walking the target's ordered endpoints.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use http::{HeaderName, HeaderValue, StatusCode};
use switchyard_llm_client::{
    Backend, ClientRouter, HttpBackendConfig, ModelConfig, TranslatingLlmClient,
};
use switchyard_protocol::{LlmClientError, ModelId, Request, Response, RoutedLlmClient};

use crate::config::{Config, Endpoint, Provider};

/// Response header naming the provider endpoint that served a request.
pub const PROVIDER_HEADER: &str = "x-conductor-provider";

struct EndpointClient {
    provider: String,
    model: String,
    client: TranslatingLlmClient,
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
        let mut last_error = None;
        for (index, endpoint) in self.endpoints.iter().enumerate() {
            let mut attempt = request.clone();
            attempt.llm_request.model = Some(endpoint.model.clone());
            match endpoint.client.call(attempt).await {
                Ok(mut response) => {
                    if let Ok(value) = HeaderValue::from_str(&endpoint.provider) {
                        response
                            .upstream_headers
                            .insert(HeaderName::from_static(PROVIDER_HEADER), value);
                    }
                    return Ok(response);
                }
                Err(error) if fails_over(&error) && index + 1 < self.endpoints.len() => {
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
}

fn endpoint_client(provider: &Provider, endpoint: &Endpoint) -> Result<TargetEndpoint, String> {
    let backend = Backend::OpenAiChat(HttpBackendConfig {
        base_url: provider.base_url.clone(),
        api_key: Some(provider.api_key.clone()),
        forward_auth: false,
        extra_headers: provider.headers.clone(),
        extra_body: Default::default(),
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
        client,
    })
}

type TargetEndpoint = EndpointClient;

/// Builds the router that maps every configured target to its client.
pub fn build(config: &Config) -> Result<ClientRouter, String> {
    let mut by_model: HashMap<ModelId, Arc<dyn RoutedLlmClient>> = HashMap::new();
    for (id, endpoints) in &config.targets {
        let clients = endpoints
            .iter()
            .map(|endpoint| endpoint_client(&config.providers[&endpoint.provider], endpoint))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("target `{id}`: {e}"))?;
        by_model.insert(
            ModelId::from(id.as_str()),
            Arc::new(TargetClient { endpoints: clients }),
        );
    }
    Ok(ClientRouter::new(by_model))
}
