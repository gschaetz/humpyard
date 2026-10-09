//! Gateway errors, rendered in the error shape of the endpoint the client called.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use switchyard_libsy::LibsyError;
use switchyard_protocol::{LlmClientError, WireFormat};

use crate::config::FallbackClass;

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("{0}")]
    BadRequest(String),
    #[error("model `{0}` not found")]
    ModelNotFound(String),
    /// Upstream returned a client error (4xx, including 429); status and message pass through.
    #[error("{message}")]
    Upstream { status: StatusCode, message: String },
    #[error("upstream unreachable: {0}")]
    UpstreamUnreachable(String),
    #[error("upstream timed out")]
    UpstreamTimeout,
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    BudgetExceeded(String),
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    Internal(String),
}

impl GatewayError {
    pub fn status(&self) -> StatusCode {
        match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::ModelNotFound(_) => StatusCode::NOT_FOUND,
            Self::Upstream { status, .. } => *status,
            Self::UpstreamUnreachable(_) => StatusCode::BAD_GATEWAY,
            Self::UpstreamTimeout => StatusCode::GATEWAY_TIMEOUT,
            Self::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            Self::Forbidden(_) => StatusCode::FORBIDDEN,
            Self::BudgetExceeded(_) => StatusCode::PAYMENT_REQUIRED,
            Self::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn kind(&self) -> &'static str {
        match self.status().as_u16() {
            400 => "invalid_request_error",
            401 => "authentication_error",
            403 => "permission_error",
            402 => "insufficient_quota",
            404 => "not_found_error",
            429 => "rate_limit_error",
            s if s < 500 => "invalid_request_error",
            _ => "api_error",
        }
    }

    /// The JSON error body for `format`.
    pub fn body(&self, format: WireFormat) -> serde_json::Value {
        let message = self.to_string();
        match format {
            WireFormat::AnthropicMessages => {
                json!({"type": "error", "error": {"type": self.kind(), "message": message}})
            }
            WireFormat::OpenAiChat | WireFormat::OpenAiResponses => {
                json!({"error": {"message": message, "type": self.kind(), "code": null}})
            }
        }
    }

    pub fn into_response_for(self, format: WireFormat) -> Response {
        (self.status(), Json(self.body(format))).into_response()
    }
}

impl From<LibsyError> for GatewayError {
    fn from(error: LibsyError) -> Self {
        match error {
            LibsyError::ClientCall { source, .. } => source.into(),
            LibsyError::NoTargets => Self::Unavailable("no eligible target".into()),
            other => Self::Internal(other.to_string()),
        }
    }
}

/// Body of the synthetic 504 the pool substitutes for a timed-out target. Switchyard falls back to
/// the next target on a 5xx but not on a timeout, so the pool re-labels the timeout; `From` turns
/// it back into `UpstreamTimeout`, so a client still sees a timeout when nothing else answers.
pub const TIMEOUT_MARKER: &str = "humpyard: upstream timed out";

/// Which kind of failure `error` is, for deciding whether the route may hand the request to its
/// next target. `None` for errors that never fall through (bad requests, auth failures, ...).
#[must_use]
pub fn fallback_class(error: &LlmClientError) -> Option<FallbackClass> {
    match error {
        LlmClientError::ContextWindowExceeded { .. } => Some(FallbackClass::Overflow),
        LlmClientError::Timeout { .. } => Some(FallbackClass::Timeout),
        LlmClientError::Transport { .. } => Some(FallbackClass::Connection),
        LlmClientError::UpstreamHttp { status, body } => {
            if *status == StatusCode::GATEWAY_TIMEOUT && body == TIMEOUT_MARKER {
                Some(FallbackClass::Timeout)
            } else if *status == StatusCode::TOO_MANY_REQUESTS {
                Some(FallbackClass::RateLimit)
            } else if *status == StatusCode::FORBIDDEN {
                Some(FallbackClass::Forbidden)
            } else if *status == StatusCode::REQUEST_TIMEOUT || status.is_server_error() {
                Some(FallbackClass::ServerError)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Prefix of the synthetic error that tells Switchyard to stop: it falls back on a fixed set of
/// errors, so a failure the route must not fall back on is re-labelled as a general error and
/// restored by `From` (the client still sees the original status and message).
const STOP_PREFIX: &str = "humpyard-no-fallback:";

/// Re-labels `error` (of kind `class`) so the route does not hand it to the next target.
#[must_use]
pub fn stop_fallback(class: FallbackClass, error: &LlmClientError) -> LlmClientError {
    let (status, text) = match error {
        LlmClientError::UpstreamHttp { status, body } => (status.as_u16(), body.clone()),
        LlmClientError::ContextWindowExceeded { message, .. } => (0, message.clone()),
        _ => (0, String::new()),
    };
    let class_name = format!("{class:?}");
    LlmClientError::General(format!(
        "{STOP_PREFIX}{}",
        json!({"class": class_name, "status": status, "text": text})
    ))
}

/// The original error behind a `stop_fallback` re-label.
fn restore_stopped(message: &str) -> Option<GatewayError> {
    let value: serde_json::Value = serde_json::from_str(message.strip_prefix(STOP_PREFIX)?).ok()?;
    let text = value["text"].as_str().unwrap_or_default().to_string();
    let status = u16::try_from(value["status"].as_u64().unwrap_or(0)).ok()?;
    Some(match value["class"].as_str()? {
        "Overflow" => GatewayError::BadRequest(text),
        "Timeout" => GatewayError::UpstreamTimeout,
        "Connection" => GatewayError::UpstreamUnreachable("upstream connection failed".into()),
        _ => GatewayError::from(LlmClientError::UpstreamHttp {
            status: StatusCode::from_u16(status).ok()?,
            body: text,
        }),
    })
}

impl From<LlmClientError> for GatewayError {
    fn from(error: LlmClientError) -> Self {
        match error {
            LlmClientError::General(message) if message.starts_with(STOP_PREFIX) => {
                restore_stopped(&message).unwrap_or(Self::Internal(message))
            }
            LlmClientError::UpstreamHttp { status, body }
                if status == StatusCode::GATEWAY_TIMEOUT && body == TIMEOUT_MARKER =>
            {
                Self::UpstreamTimeout
            }
            LlmClientError::UpstreamHttp { status, body } => {
                let message = upstream_message(&body, status);
                if status.is_client_error() {
                    Self::Upstream { status, message }
                } else {
                    Self::UpstreamUnreachable(format!("upstream returned {status}: {message}"))
                }
            }
            LlmClientError::Timeout { .. } => Self::UpstreamTimeout,
            // Transport errors can quote the provider URL, so the client gets a generic message.
            LlmClientError::Transport { .. } => {
                Self::UpstreamUnreachable("upstream connection failed".into())
            }
            LlmClientError::InvalidRequest { message }
            | LlmClientError::RequestTranslation(message)
            | LlmClientError::ContextWindowExceeded { message, .. } => Self::BadRequest(message),
            other => Self::UpstreamUnreachable(other.to_string()),
        }
    }
}

/// The provider's own error message when it sent a recognizable JSON shape (`error.message`,
/// `error` as a string, or `message`). Anything else, notably an HTML error page from a proxy, is
/// reduced to the status; short plain text is kept, collapsed to one clipped line.
fn upstream_message(body: &str, status: StatusCode) -> String {
    let from_json = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v["error"]["message"]
                .as_str()
                .or_else(|| v["error"].as_str())
                .or_else(|| v["message"].as_str())
                .map(str::to_string)
        });
    if let Some(message) = from_json {
        return message;
    }
    let text = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() || text.contains('<') || text.starts_with(['{', '[']) {
        format!("upstream returned {status}")
    } else {
        let clipped: String = text.chars().take(200).collect();
        format!("upstream returned {status}: {clipped}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses() {
        assert_eq!(GatewayError::BadRequest("x".into()).status(), 400);
        assert_eq!(GatewayError::ModelNotFound("m".into()).status(), 404);
        assert_eq!(
            GatewayError::Upstream {
                status: StatusCode::TOO_MANY_REQUESTS,
                message: "slow down".into()
            }
            .status(),
            429
        );
        assert_eq!(GatewayError::UpstreamUnreachable("x".into()).status(), 502);
        assert_eq!(GatewayError::UpstreamTimeout.status(), 504);
    }

    #[test]
    fn upstream_errors_map_by_class() {
        let http = |code: u16, body: &str| {
            GatewayError::from(LlmClientError::UpstreamHttp {
                status: StatusCode::from_u16(code).unwrap(),
                body: body.into(),
            })
        };
        let limited = http(429, r#"{"error":{"message":"slow down"}}"#);
        assert_eq!(limited.status(), 429);
        assert_eq!(limited.to_string(), "slow down");
        assert_eq!(http(400, "plain text").status(), 400);
        assert_eq!(http(503, "{}").status(), 502);
        assert_eq!(
            GatewayError::from(LlmClientError::Timeout { source: "t".into() }).status(),
            504
        );
        let transport = GatewayError::from(LlmClientError::Transport {
            source: "http://secret/url".into(),
        });
        assert_eq!(transport.status(), 502);
        assert!(!transport.to_string().contains("secret"));
    }

    #[test]
    fn upstream_messages_are_readable_whatever_the_body() {
        let msg = |body: &str| upstream_message(body, StatusCode::NOT_FOUND);
        assert_eq!(
            msg(r#"{"error":{"message":"no such model"}}"#),
            "no such model"
        );
        assert_eq!(msg(r#"{"error":"flat string"}"#), "flat string");
        assert_eq!(msg(r#"{"message":"top level"}"#), "top level");
        let html = "<!DOCTYPE HTML>\n<title>404 Not Found</title>\n<h1>Not Found</h1>";
        assert_eq!(msg(html), "upstream returned 404 Not Found");
        assert_eq!(msg(""), "upstream returned 404 Not Found");
        assert_eq!(
            msg(r#"{"unknown":"shape"}"#),
            "upstream returned 404 Not Found"
        );
        assert_eq!(
            msg("model\n  is   overloaded"),
            "upstream returned 404 Not Found: model is overloaded"
        );
        let long = "x".repeat(900);
        assert!(msg(&long).len() < 260);
    }

    #[test]
    fn failures_are_classified_for_fallback() {
        let http = |code: u16, body: &str| LlmClientError::UpstreamHttp {
            status: StatusCode::from_u16(code).unwrap(),
            body: body.into(),
        };
        assert_eq!(
            fallback_class(&http(429, "")),
            Some(FallbackClass::RateLimit)
        );
        assert_eq!(
            fallback_class(&http(403, "")),
            Some(FallbackClass::Forbidden)
        );
        assert_eq!(
            fallback_class(&http(408, "")),
            Some(FallbackClass::ServerError)
        );
        assert_eq!(
            fallback_class(&http(502, "")),
            Some(FallbackClass::ServerError)
        );
        assert_eq!(
            fallback_class(&http(504, TIMEOUT_MARKER)),
            Some(FallbackClass::Timeout)
        );
        assert_eq!(fallback_class(&http(400, "")), None);
        assert_eq!(fallback_class(&http(404, "")), None);
        assert_eq!(
            fallback_class(&LlmClientError::Timeout { source: "t".into() }),
            Some(FallbackClass::Timeout)
        );
        assert_eq!(
            fallback_class(&LlmClientError::Transport { source: "t".into() }),
            Some(FallbackClass::Connection)
        );
        assert_eq!(fallback_class(&LlmClientError::General("x".into())), None);
    }

    #[test]
    fn a_stopped_failure_comes_back_as_the_original() {
        let original = LlmClientError::UpstreamHttp {
            status: StatusCode::TOO_MANY_REQUESTS,
            body: r#"{"error":{"message":"slow down"}}"#.into(),
        };
        let stopped = stop_fallback(FallbackClass::RateLimit, &original);
        assert!(
            matches!(stopped, LlmClientError::General(_)),
            "must not look like a fallback error"
        );
        let restored = GatewayError::from(stopped);
        assert_eq!(restored.status(), 429);
        assert_eq!(restored.to_string(), "slow down");

        let timeout = stop_fallback(
            FallbackClass::Timeout,
            &LlmClientError::Timeout { source: "t".into() },
        );
        assert_eq!(GatewayError::from(timeout).status(), 504);
        let overflow = stop_fallback(
            FallbackClass::Overflow,
            &LlmClientError::ContextWindowExceeded {
                model: "m".into(),
                message: "too long".into(),
            },
        );
        let restored = GatewayError::from(overflow);
        assert_eq!(
            (restored.status().as_u16(), restored.to_string()),
            (400, "too long".into())
        );
    }

    #[test]
    fn openai_shape() {
        let body = GatewayError::ModelNotFound("m".into()).body(WireFormat::OpenAiChat);
        assert_eq!(body["error"]["type"], "not_found_error");
        assert_eq!(body["error"]["message"], "model `m` not found");
    }

    #[test]
    fn anthropic_shape() {
        let body = GatewayError::BadRequest("bad".into()).body(WireFormat::AnthropicMessages);
        assert_eq!(body["type"], "error");
        assert_eq!(body["error"]["type"], "invalid_request_error");
        assert_eq!(body["error"]["message"], "bad");
    }
}
