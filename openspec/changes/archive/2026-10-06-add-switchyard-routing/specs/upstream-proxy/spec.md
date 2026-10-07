# Spec Delta

## MODIFIED Requirements

### Requirement: Forward to configured upstream
The gateway SHALL send each request to the selected endpoint's provider at its OpenAI-compatible chat completions URL with that provider's API key as a bearer token.

#### Scenario: Request forwarded with credentials
- **WHEN** a valid request is routed to an endpoint
- **THEN** the upstream call carries `Authorization: Bearer <key>` for that endpoint's provider and the endpoint's model name

### Requirement: Upstream error mapping
The gateway SHALL map upstream failures to client-visible errors once failover is exhausted: upstream 4xx and 429 pass through status and message; connection errors and timeouts return 502 and 504.

#### Scenario: Upstream rate limit
- **WHEN** every endpoint returns HTTP 429
- **THEN** the client receives HTTP 429 with the last upstream message in the endpoint's error format

#### Scenario: Upstream unreachable
- **WHEN** no endpoint can be reached
- **THEN** the client receives HTTP 502
