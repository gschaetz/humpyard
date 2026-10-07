# Spec Delta

## Purpose

Defines the HTTP surface clients use to talk to the gateway, including which LLM protocols are
accepted and how failures are reported back to the client.

## ADDED Requirements

### Requirement: OpenAI Chat Completions endpoint
The gateway SHALL accept `POST /v1/chat/completions` requests in OpenAI Chat Completions format.

#### Scenario: Non-streaming chat completion
- **WHEN** a client posts a valid chat completion request with `stream` absent or false
- **THEN** the gateway returns HTTP 200 with a single OpenAI-format JSON response

#### Scenario: Streaming chat completion
- **WHEN** a client posts a valid request with `stream: true`
- **THEN** the gateway returns `text/event-stream` OpenAI-format chunks ending with `data: [DONE]`

### Requirement: Anthropic Messages endpoint
The gateway SHALL accept `POST /v1/messages` requests in Anthropic Messages format and reply in
the same format, regardless of the upstream's protocol.

#### Scenario: Anthropic client against an OpenAI-compatible upstream
- **WHEN** a client posts an Anthropic Messages request
- **THEN** the gateway returns an Anthropic Messages response (or Anthropic SSE events when streaming)

### Requirement: Model listing
The gateway SHALL answer `GET /v1/models` with an OpenAI-format list containing the configured model names.

#### Scenario: List models
- **WHEN** a client requests `/v1/models`
- **THEN** the response lists each model name from the configuration

### Requirement: Health endpoint
The gateway SHALL answer `GET /healthz` with HTTP 200 while the process is serving.

#### Scenario: Liveness check
- **WHEN** `/healthz` is requested
- **THEN** the gateway returns HTTP 200

### Requirement: Protocol-shaped errors
The gateway SHALL return errors in the protocol of the endpoint the client called.

#### Scenario: Malformed request body
- **WHEN** a client posts invalid JSON to either endpoint
- **THEN** the gateway returns HTTP 400 with an error body in that endpoint's error format

#### Scenario: Unknown model
- **WHEN** a request names a model not in the configuration
- **THEN** the gateway returns HTTP 404 with a model-not-found error in the endpoint's format
