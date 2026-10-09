# upstream-proxy Specification

## Purpose
Defines how the gateway forwards requests to the configured upstream provider and relays the
result, including streaming and failure handling.

## Requirements

### Requirement: Forward to configured upstream
The gateway SHALL send each request to the selected endpoint's provider at its OpenAI-compatible chat completions URL with that provider's API key as a bearer token.

#### Scenario: Request forwarded with credentials
- **WHEN** a valid request is routed to an endpoint
- **THEN** the upstream call carries `Authorization: Bearer <key>` for that endpoint's provider and the endpoint's model name

### Requirement: Incremental streaming
The gateway SHALL forward streamed chunks to the client as they arrive, without buffering the whole response.

#### Scenario: First token latency
- **WHEN** the upstream emits its first chunk
- **THEN** the gateway writes the translated chunk to the client before the upstream finishes

### Requirement: Client disconnect cancels upstream
The gateway SHALL abort the upstream request when the client disconnects.

#### Scenario: Client drops mid-stream
- **WHEN** the client closes the connection during a streamed response
- **THEN** the upstream request is cancelled and no further data is read

### Requirement: Upstream error mapping
The gateway SHALL map upstream failures to client-visible errors once failover is exhausted: upstream 4xx and 429 pass through status and message; connection errors and timeouts return 502 and 504.

#### Scenario: Upstream rate limit
- **WHEN** every endpoint returns HTTP 429
- **THEN** the client receives HTTP 429 with the last upstream message in the endpoint's error format

#### Scenario: Upstream unreachable
- **WHEN** no endpoint can be reached
- **THEN** the client receives HTTP 502

### Requirement: Secrets are not logged
The gateway SHALL NOT write API keys or Authorization headers to logs.

#### Scenario: Debug logging enabled
- **WHEN** request logging runs at debug level
- **THEN** no log line contains the upstream API key

### Requirement: Upstream error messages are readable
When an upstream error is passed to the client or logged, the gateway SHALL use the provider's JSON error message when one is present, and otherwise a short status-based message that never includes HTML or unrecognized JSON bodies.

#### Scenario: HTML error page
- **WHEN** an upstream answers 404 with an HTML page
- **THEN** the client's message is `upstream returned 404 Not Found`

#### Scenario: Provider JSON message
- **WHEN** the upstream body has `error.message`
- **THEN** that message is passed through

### Requirement: Requests are adapted for chat-completions providers
Before sending a request upstream the gateway SHALL remove Responses-shaped reasoning details (entries of type `reasoning`) while keeping the reasoning text and chat-style details, and SHALL send the `developer` role as `system`.

#### Scenario: Replayed reasoning item
- **WHEN** a Responses request replays a reasoning item that carries `content` or `encrypted_content`
- **THEN** the upstream request has no Responses-shaped `reasoning_details` and still carries the reasoning text

#### Scenario: Chat-style details are untouched
- **WHEN** a chat request carries `reasoning_details` of type `reasoning.encrypted`
- **THEN** they reach the provider unchanged

#### Scenario: Developer messages
- **WHEN** a request contains developer-role messages, at the start or mid-conversation
- **THEN** the upstream request has no `developer` role and the same number of system messages
