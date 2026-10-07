//! Thin wrapper over Switchyard's translation engine. All format conversion goes through here.

use serde_json::Value;
use switchyard_protocol::WireFormat;
use switchyard_translation::{StreamTranslationState, TranslationEngine, TranslationPolicy};

use crate::error::GatewayError;

/// The upstream always speaks OpenAI Chat Completions.
pub const UPSTREAM_FORMAT: WireFormat = WireFormat::OpenAiChat;

#[derive(Default)]
pub struct Translator {
    engine: TranslationEngine,
    policy: TranslationPolicy,
}

fn failed(error: impl std::fmt::Display) -> GatewayError {
    GatewayError::BadRequest(format!("cannot translate request: {error}"))
}

impl Translator {
    /// Client request body to the upstream's OpenAI Chat body.
    pub fn request_to_upstream(
        &self,
        client: WireFormat,
        body: &Value,
    ) -> Result<Value, GatewayError> {
        self.engine
            .translate_request(client, UPSTREAM_FORMAT, body, &self.policy)
            .map(|out| out.body)
            .map_err(failed)
    }

    /// Upstream OpenAI Chat response to the client's format.
    pub fn response_to_client(
        &self,
        client: WireFormat,
        body: &Value,
    ) -> Result<Value, GatewayError> {
        self.engine
            .translate_response(UPSTREAM_FORMAT, client, body, &self.policy)
            .map(|out| out.body)
            .map_err(|e| GatewayError::Internal(format!("cannot translate upstream response: {e}")))
    }

    /// One upstream stream event to zero or more client events.
    pub fn event_to_client(
        &self,
        state: &mut StreamTranslationState,
        client: WireFormat,
        event: &Value,
    ) -> Result<Vec<Value>, GatewayError> {
        self.engine
            .translate_event(state, UPSTREAM_FORMAT, client, event)
            .map_err(|e| GatewayError::Internal(format!("cannot translate stream event: {e}")))
    }

    /// Trailing client events once the upstream stream has closed.
    pub fn finish_stream(
        &self,
        state: &mut StreamTranslationState,
        client: WireFormat,
    ) -> Result<Vec<Value>, GatewayError> {
        self.engine
            .finish_stream(state, client)
            .map_err(|e| GatewayError::Internal(format!("cannot finish stream: {e}")))
    }
}
