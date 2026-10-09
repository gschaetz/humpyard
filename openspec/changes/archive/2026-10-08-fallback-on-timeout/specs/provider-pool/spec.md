# Spec Delta

## ADDED Requirements

### Requirement: Cross-target fallback on timeout
When all endpoints of a target fail and the last failure was a timeout, the gateway SHALL try the route's next target, and SHALL return the usual 504 timeout error if none succeeds.

#### Scenario: Timeout then fallback
- **WHEN** the first target times out and the second answers
- **THEN** the request is served by the second target

#### Scenario: Nothing to fall back to
- **WHEN** the only target times out
- **THEN** the client receives 504 with the message `upstream timed out`

### Requirement: Fallback matrix is pinned
Context-window overflow, 429, 403 and 5xx SHALL fall back to the next target; other client errors (400, 401, 404) SHALL stop at once and reach the client.

#### Scenario: Client error does not fall back
- **WHEN** the first target answers 404
- **THEN** the client receives 404 and the second target is not contacted
