//! HTTP surface: inference endpoints in three client protocols, all proxied to one upstream.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures::StreamExt;
use serde_json::{Value, json};
use switchyard_protocol::WireFormat;
use switchyard_translation::StreamTranslationState;

use crate::config::Config;
use crate::error::GatewayError;
use crate::translate::Translator;

pub struct AppState {
    config: Config,
    client: reqwest::Client,
    translator: Translator,
}

pub fn router(config: Config) -> Router {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("reqwest client builds with default TLS");
    let state = Arc::new(AppState {
        config,
        client,
        translator: Translator::default(),
    });
    Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .route("/v1/models", get(list_models))
        .route(
            "/v1/chat/completions",
            post(|s, b| infer(s, WireFormat::OpenAiChat, b)),
        )
        .route(
            "/v1/responses",
            post(|s, b| infer(s, WireFormat::OpenAiResponses, b)),
        )
        .route(
            "/v1/messages",
            post(|s, b| infer(s, WireFormat::AnthropicMessages, b)),
        )
        .with_state(state)
}

async fn list_models(State(state): State<Arc<AppState>>) -> Response {
    let data: Vec<Value> = state
        .config
        .model_names()
        .into_iter()
        .map(|id| json!({"id": id, "object": "model", "created": 0, "owned_by": "switchyard-conductor"}))
        .collect();
    axum::Json(json!({"object": "list", "data": data})).into_response()
}

async fn infer(State(state): State<Arc<AppState>>, format: WireFormat, body: Bytes) -> Response {
    let started = std::time::Instant::now();
    let result = handle(&state, format, &body).await;
    match result {
        Ok((model, response)) => {
            tracing::info!(%format, model, status = response.status().as_u16(), elapsed_ms = started.elapsed().as_millis() as u64, "request");
            response
        }
        Err(error) => {
            tracing::warn!(%format, status = error.status().as_u16(), error = %error, "request failed");
            error.into_response_for(format)
        }
    }
}

async fn handle(
    state: &AppState,
    format: WireFormat,
    raw: &[u8],
) -> Result<(String, Response), GatewayError> {
    let body: Value = serde_json::from_slice(raw)
        .map_err(|e| GatewayError::BadRequest(format!("invalid JSON body: {e}")))?;
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| GatewayError::BadRequest("`model` is required".into()))?
        .to_string();
    // Routes are accepted in config but not served until the routing layer lands; only direct
    // targets are requestable for now, via their first endpoint.
    let endpoint = match state.config.targets.get(&model) {
        Some(endpoints) => &endpoints[0],
        None if state.config.routes.contains_key(&model) => {
            return Err(GatewayError::BadRequest(format!(
                "route `{model}` is configured but routing is not enabled yet; request a target directly"
            )));
        }
        None => return Err(GatewayError::ModelNotFound(model)),
    };
    let provider = &state.config.providers[&endpoint.provider];
    if format == WireFormat::OpenAiResponses
        && body
            .get("previous_response_id")
            .is_some_and(|v| !v.is_null())
    {
        return Err(GatewayError::BadRequest(
            "`previous_response_id` is not supported: stateful responses are unavailable".into(),
        ));
    }
    let stream = body.get("stream").and_then(Value::as_bool).unwrap_or(false);

    let mut upstream_body = state.translator.request_to_upstream(format, &body)?;
    upstream_body["model"] = Value::String(endpoint.model.clone());
    upstream_body["stream"] = Value::Bool(stream);
    if stream {
        upstream_body["stream_options"] = json!({"include_usage": true});
    }

    let response = state
        .client
        .post(format!("{}/chat/completions", provider.base_url))
        .bearer_auth(&provider.api_key)
        .timeout(Duration::from_secs(provider.timeout_secs))
        .json(&upstream_body)
        .send()
        .await
        .map_err(send_error)?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(upstream_error(status, &text));
    }

    if stream {
        return Ok((model, stream_response(format, response)));
    }
    let upstream_json: Value = response.json().await.map_err(send_error)?;
    let client_json = state
        .translator
        .response_to_client(format, &upstream_json)?;
    Ok((model, axum::Json(client_json).into_response()))
}

fn send_error(error: reqwest::Error) -> GatewayError {
    if error.is_timeout() {
        GatewayError::UpstreamTimeout
    } else {
        // Strip the URL so nothing configuration-specific leaks into client errors.
        GatewayError::UpstreamUnreachable(error.without_url().to_string())
    }
}

fn upstream_error(status: StatusCode, text: &str) -> GatewayError {
    let message = serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(str::to_string))
        .unwrap_or_else(|| {
            if text.is_empty() {
                format!("upstream returned {status}")
            } else {
                text.chars().take(500).collect()
            }
        });
    if status.is_client_error() {
        GatewayError::Upstream { status, message }
    } else {
        GatewayError::UpstreamUnreachable(format!("upstream returned {status}: {message}"))
    }
}

/// Relays the upstream SSE stream, translating each event. Dropping the returned body drops the
/// upstream connection, which cancels the upstream request.
fn stream_response(format: WireFormat, response: reqwest::Response) -> Response {
    // The translator is stateless apart from `StreamTranslationState`, so a fresh one is cheap
    // and keeps the stream `'static` without sharing `AppState`.
    let translator = Translator::default();
    let mut upstream = response.bytes_stream();
    let events = async_stream::stream! {
        let mut state = StreamTranslationState::default();
        let mut buffer = String::new();
        while let Some(chunk) = upstream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    yield Ok::<Event, Infallible>(error_event(format, &error.without_url().to_string()));
                    return;
                }
            };
            buffer.push_str(&String::from_utf8_lossy(&chunk).replace("\r\n", "\n"));
            while let Some(end) = buffer.find("\n\n") {
                let block: String = buffer.drain(..end + 2).collect();
                let Some(data) = sse_data(&block) else { continue };
                if data == "[DONE]" {
                    continue;
                }
                let Ok(value) = serde_json::from_str::<Value>(&data) else { continue };
                match translator.event_to_client(&mut state, format, &value) {
                    Ok(out) => for v in out { yield Ok(to_event(format, &v)); },
                    Err(error) => { yield Ok(error_event(format, &error.to_string())); return; }
                }
            }
        }
        match translator.finish_stream(&mut state, format) {
            Ok(out) => for v in out { yield Ok(to_event(format, &v)); },
            Err(error) => { yield Ok(error_event(format, &error.to_string())); return; }
        }
        if format == WireFormat::OpenAiChat {
            yield Ok(Event::default().data("[DONE]"));
        }
    };
    Sse::new(events).into_response()
}

fn sse_data(block: &str) -> Option<String> {
    let lines: Vec<&str> = block
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(|l| l.strip_prefix(' ').unwrap_or(l))
        .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// Anthropic and Responses clients expect a named SSE event matching the payload's `type`.
fn to_event(format: WireFormat, value: &Value) -> Event {
    let event = Event::default();
    let event = match (format, value["type"].as_str()) {
        (WireFormat::OpenAiChat, _) | (_, None) => event,
        (_, Some(name)) => event.event(name),
    };
    event.data(value.to_string())
}

fn error_event(format: WireFormat, message: &str) -> Event {
    let body = GatewayError::UpstreamUnreachable(message.to_string()).body(format);
    to_event(format, &body).event("error")
}
