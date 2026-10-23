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

### Requirement: Responses input-token counting endpoint
The gateway SHALL answer `POST /v1/responses/input_tokens` with `{"object": "response.input_tokens", "input_tokens": n}`, where n is the same kind of local estimate as the Anthropic counting endpoint, marked with the `x-humpyard-token-count: estimate` header.

#### Scenario: Count a Responses request
- **WHEN** a client posts a Responses request body (`model`, `input`) to `/v1/responses/input_tokens`
- **THEN** the response is 200 with the object `response.input_tokens`, a positive `input_tokens` and the estimate header

#### Scenario: More input counts more
- **WHEN** the request carries more input items, instructions or tool definitions
- **THEN** its `input_tokens` is greater than that of the shorter request

### Requirement: Responses counting shares the counting rules
Responses token counting SHALL make no upstream call, write no ledger entry, remain available to a key whose budget is exhausted, and follow the authentication, allowlist and unknown-model rules of the Responses endpoint, with errors in the Responses error format.

#### Scenario: Unauthenticated
- **WHEN** keys are configured and a request has none
- **THEN** the response is 401 in the OpenAI error format

#### Scenario: Stateful request
- **WHEN** the body carries `previous_response_id`
- **THEN** the response is 400, as for `/v1/responses`

### Requirement: Session id fallbacks
When none of the headers the routing engine recognizes as a session id carries a usable one, the gateway SHALL take the session id from the `session_id` header, then from `x-session-affinity`. A usable id is 1 to 256 visible ASCII characters; any other value SHALL be ignored without failing the request. The id SHALL be used wherever a session id is used (the decision, the usage rows and the routing engine's per-session state).

#### Scenario: A client that sends session_id
- **WHEN** a request carries `session_id: abc` and no recognized session header
- **THEN** its decision and usage rows carry the session `abc` and the routing engine sees it

#### Scenario: Recognized headers win
- **WHEN** a request carries both `x-switchyard-session-id: one` and `session_id: two`
- **THEN** the session is `one`

#### Scenario: Header order among the fallbacks
- **WHEN** a request carries both `session_id: first` and `x-session-affinity: second` and no recognized header
- **THEN** the session is `first`

#### Scenario: Unusable values
- **WHEN** the value is empty, longer than 256 characters, or contains a space or control character
- **THEN** it is ignored, the next candidate is tried, and the request is served normally

#### Scenario: Not a session
- **WHEN** a request carries only `x-client-request-id`
- **THEN** it has no session id

### Requirement: The effective configuration is readable with provenance
The gateway SHALL provide `GET /v1/config`, which returns the configuration it is serving with secrets omitted, the fingerprint and load time of that configuration, and for every target and route its source, `file` or `generated`. It SHALL NOT include any API key value, any key hash or any provider header value.

#### Scenario: A value from each source
- **WHEN** the endpoint is read while a generated target exists
- **THEN** a hand-written target is labelled `file` and the generated target `generated`

#### Scenario: Pool routes show what they resolved to
- **WHEN** the endpoint is read and a route is a pool
- **THEN** the route lists its resolved member targets

#### Scenario: No secrets
- **WHEN** a provider has an API key and a header value
- **THEN** the response says whether a key is set and names the header, but contains neither the key nor the header value

### Requirement: Reading the configuration is opt-in per key
A key SHALL be able to read the effective configuration only if it sets `read_config = true`; any other key SHALL receive HTTP 403 with the error code `config_not_readable`. A gateway with no keys configured SHALL allow the read.

#### Scenario: An ordinary key
- **WHEN** a key without `read_config` requests the endpoint
- **THEN** the answer is 403 `config_not_readable` and nothing of the configuration is returned

#### Scenario: An operator key
- **WHEN** a key with `read_config = true` requests the endpoint
- **THEN** the configuration is returned

#### Scenario: The fingerprint follows a reload
- **WHEN** the configuration file changes and the gateway reloads it
- **THEN** the fingerprint in the response is the new one, equal to the one in `/v1/health`

### Requirement: Overrides are created, listed and removed through the API
The gateway SHALL provide `POST /v1/overrides`, `GET /v1/overrides` (with `?history=true`, which also lists the overrides of the last 30 days that have ended, each with its status and when it ended) and `DELETE /v1/overrides/{id}`. A create SHALL name a kind (`disable_target`, `drain_provider` or `pin_route`), its subject, a reason and optionally `expires_in_secs`; it SHALL return the override with its id and expiry. A removal SHALL take effect for the next request.

#### Scenario: Create and list
- **WHEN** a key with `manage_overrides` creates a `disable_target` for a target and then lists overrides
- **THEN** the override appears with its kind, subject, reason, creator id and expiry

#### Scenario: Remove
- **WHEN** an override is removed
- **THEN** the next request is planned without it and the removal is in the audit trail

### Requirement: Writing overrides needs an opt-in key
Creating or removing an override SHALL require a key with `manage_overrides = true`; any other key SHALL receive HTTP 403 with the error code `overrides_not_manageable`. Listing SHALL require `manage_overrides` or `read_config`. A gateway with no keys configured SHALL allow all of it.

#### Scenario: An ordinary key
- **WHEN** a key without the setting tries to create an override
- **THEN** the answer is 403 `overrides_not_manageable` and nothing is stored

### Requirement: Overrides must be bounded and must name existing things
A create without a ledger SHALL be refused with 409. An expiry SHALL be required (a default applies when omitted) and SHALL NOT exceed the configured maximum. A subject that is not in the loaded configuration SHALL be refused, and a `pin_route` to a target that is not a candidate of that route, or whose class a strict route forbids, SHALL be refused.

#### Scenario: Too long
- **WHEN** an override asks for an expiry beyond the maximum
- **THEN** it is refused and nothing is stored

#### Scenario: Unknown subject
- **WHEN** an override names a target that does not exist
- **THEN** it is refused

#### Scenario: A pin cannot cross a class
- **WHEN** a private-only route is pinned to a public target
- **THEN** it is refused
