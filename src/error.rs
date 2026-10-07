//! Gateway errors, rendered in the error shape of the endpoint the client called.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use switchyard_protocol::WireFormat;

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
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn kind(&self) -> &'static str {
        match self.status().as_u16() {
            400 => "invalid_request_error",
            401 | 403 => "authentication_error",
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
