# gateway-api Specification

## Purpose
Defines the HTTP surface clients use to talk to the gateway, including which LLM protocols are
accepted and how failures are reported back to the client.

## Requirements

### Requirement: OpenAI Chat Completions endpoint
The gateway SHALL accept `POST /v1/chat/completions` requests in OpenAI Chat Completions format.

#### Scenario: Non-streaming chat completion
- **WHEN** a client posts a valid chat completion request with `stream` absent or false
- **THEN** the gateway returns HTTP 200 with a single OpenAI-format JSON response

#### Scenario: Streaming chat completion
- **WHEN** a client posts a valid request with `stream: true`
- **THEN** the gateway returns `text/event-stream` OpenAI-format chunks ending with `data: [DONE]`

### Requirement: OpenAI Responses endpoint
The gateway SHALL accept `POST /v1/responses` requests in OpenAI Responses format, so Codex-style clients can use it, and reply in the same format regardless of the upstream's protocol.

#### Scenario: Non-streaming Responses request
- **WHEN** a client posts a valid Responses request with `stream` absent or false
- **THEN** the gateway returns HTTP 200 with a single Responses-format JSON object

#### Scenario: Streaming Responses request
- **WHEN** a client posts a valid Responses request with `stream: true`
- **THEN** the gateway returns Responses-format SSE events (`response.created` through `response.completed`)

#### Scenario: Tool-calling Responses request
- **WHEN** a Responses request includes function tools and the upstream returns a tool call
- **THEN** the client receives a Responses `function_call` output item with matching call id and arguments

### Requirement: Anthropic Messages endpoint
The gateway SHALL accept `POST /v1/messages` requests in Anthropic Messages format and reply in
the same format, regardless of the upstream's protocol.

#### Scenario: Anthropic client against an OpenAI-compatible upstream
- **WHEN** a client posts an Anthropic Messages request
- **THEN** the gateway returns an Anthropic Messages response (or Anthropic SSE events when streaming)

### Requirement: Model listing
The gateway SHALL answer `GET /v1/models` with an OpenAI-format list containing every configured route and target name.

#### Scenario: List models
- **WHEN** a client requests `/v1/models`
- **THEN** the response lists each route and target name from the configuration

### Requirement: Health endpoint
The gateway SHALL answer `GET /healthz` with HTTP 200 while the process is serving.

#### Scenario: Liveness check
- **WHEN** `/healthz` is requested
- **THEN** the gateway returns HTTP 200

### Requirement: Protocol-shaped errors
The gateway SHALL return errors in the protocol of the endpoint the client called.

#### Scenario: Malformed request body
- **WHEN** a client posts invalid JSON to any inference endpoint
- **THEN** the gateway returns HTTP 400 with an error body in that endpoint's error format

#### Scenario: Unknown model
- **WHEN** a request names a model not in the configuration
- **THEN** the gateway returns HTTP 404 with a model-not-found error in the endpoint's format

### Requirement: Unsupported stateful Responses features
The gateway SHALL reject Responses requests that depend on server-side state, such as `previous_response_id`, with HTTP 400.

#### Scenario: Previous response referenced
- **WHEN** a Responses request includes `previous_response_id`
- **THEN** the gateway returns HTTP 400 explaining that stateful responses are unsupported

### Requirement: Token counting endpoint
The gateway SHALL answer `POST /v1/messages/count_tokens` with an Anthropic-format body `{"input_tokens": n}`, where n is a local estimate of the prompt tokens the requested model would receive, and SHALL mark the response with an `x-humpyard-token-count: estimate` header.

#### Scenario: Count a conversation
- **WHEN** a client posts an Anthropic Messages request body (without `max_tokens`) to `/v1/messages/count_tokens`
- **THEN** the response is 200 with a positive `input_tokens` and the estimate header

#### Scenario: More content counts more
- **WHEN** a request adds more messages or tool definitions
- **THEN** its `input_tokens` is greater than that of the shorter request

### Requirement: Counting makes no upstream call and costs nothing
Token counting SHALL NOT call any provider, SHALL NOT create ledger entries, and SHALL remain available to a key whose budget is exhausted.

#### Scenario: Exhausted key
- **WHEN** a key at its budget limit calls `/v1/messages/count_tokens`
- **THEN** it receives a count rather than a 402, and no ledger entry is written

### Requirement: Counting follows the same access rules as messages
The endpoint SHALL require authentication when keys are configured, SHALL apply the key's route allowlist, and SHALL answer an unknown model with 404, all in Anthropic's error shape.

#### Scenario: Disallowed model
- **WHEN** a key allowed only `auto` asks to count tokens for `smart`
- **THEN** the response is 403 in the Anthropic error format
