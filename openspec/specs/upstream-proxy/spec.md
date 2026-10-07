# upstream-proxy Specification

## Purpose
Defines how the gateway forwards requests to the configured upstream provider and relays the
result, including streaming and failure handling.

## Requirements

### Requirement: Forward to configured upstream
The gateway SHALL send each request to the configured upstream's OpenAI-compatible chat completions URL with the configured API key as a bearer token.

#### Scenario: Request forwarded with credentials
- **WHEN** a valid request is received for a configured model
- **THEN** the upstream call carries `Authorization: Bearer <key>` and the model name the upstream expects

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
The gateway SHALL map upstream failures to client-visible errors: upstream 4xx and 429 pass through status and message; connection errors and timeouts return 502 and 504.

#### Scenario: Upstream rate limit
- **WHEN** the upstream returns HTTP 429
- **THEN** the client receives HTTP 429 with the upstream message in the endpoint's error format

#### Scenario: Upstream unreachable
- **WHEN** the upstream cannot be reached
- **THEN** the client receives HTTP 502

### Requirement: Secrets are not logged
The gateway SHALL NOT write API keys or Authorization headers to logs.

#### Scenario: Debug logging enabled
- **WHEN** request logging runs at debug level
- **THEN** no log line contains the upstream API key
