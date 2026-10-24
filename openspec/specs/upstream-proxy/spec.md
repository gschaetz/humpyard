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

### Requirement: Failover on unavailable endpoints
A 401 or 404 from an endpoint SHALL move the request to the next endpoint of the same target for every route. A fallback class `unavailable` (401 and 404) SHALL move a request to the next target; it SHALL be included by default for `fastest` routes and pool routes, SHALL NOT be included by default for any other route, and SHALL be taken exactly as written when a route lists its `fallback_on` classes.

#### Scenario: A dead first target in a fastest pool
- **WHEN** a `fastest` pool's first target answers 404 and another target is healthy
- **THEN** the request is served by the healthy target, and after `failure_threshold` such answers the dead target is skipped

#### Scenario: Other routes keep their behavior
- **WHEN** a `passthrough` route without `unavailable` in `fallback_on` has a first target that answers 404
- **THEN** the 404 is returned to the client and the second target is not called

#### Scenario: Opting in
- **WHEN** a route lists `fallback_on = ["unavailable"]` and its first target answers 401
- **THEN** the request moves to the next target

#### Scenario: Within a target
- **WHEN** a target has two endpoints and the first answers 404
- **THEN** the second is tried, on every route

#### Scenario: Strict routes
- **WHEN** an unavailable target is on a route with `allowed_classes`
- **THEN** fallback goes only to targets of an allowed class

#### Scenario: The client still gets an error when nothing works
- **WHEN** every target answers 404
- **THEN** the client receives the last 404

### Requirement: A provider may have request fields removed
A provider MAY set `drop_fields`, a list of top-level request field names, and the gateway SHALL remove those fields from every request it sends to that provider's endpoints, including from a preserved same-format request body. A provider that sets nothing SHALL receive the request unchanged by this rule, and a field not present SHALL be ignored.

#### Scenario: A strict provider is sent no cache key
- **WHEN** a Responses request carrying `prompt_cache_key` and `store` is routed to a provider whose `drop_fields` lists both
- **THEN** the upstream body has neither field and every other field is unchanged

#### Scenario: Other providers still get the cache key
- **WHEN** the same request is routed to a provider with no `drop_fields`
- **THEN** the upstream body still carries `prompt_cache_key`

#### Scenario: A chat client is covered too
- **WHEN** a chat-completions request carrying a listed field is routed to that provider
- **THEN** the upstream body does not carry it

#### Scenario: Failover applies each provider's own list
- **WHEN** a request fails over from a provider with `drop_fields` to one without
- **THEN** the second provider receives the field

### Requirement: An error array carries the provider's message
When an upstream error body is a JSON array whose first element has the `error.message` shape, the gateway SHALL use that message as it does for a bare object.

#### Scenario: Google-style error array
- **WHEN** the upstream body is `[{"error": {"message": "Unknown name \"store\""}}]`
- **THEN** the client's error message is `Unknown name "store"`
