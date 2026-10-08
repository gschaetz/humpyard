# Spec Delta

## ADDED Requirements

### Requirement: Endpoints that keep failing are skipped for a cooldown
After `failure_threshold` consecutive failover-class failures an endpoint SHALL be skipped for `cooldown_secs`, with the cooldown doubling on each repeated trip up to `max_cooldown_secs`. A success SHALL reset the endpoint to healthy.

#### Scenario: Breaker opens
- **WHEN** the first endpoint of a target fails `failure_threshold` times in a row
- **THEN** the next request goes straight to the next endpoint without contacting the failed one

#### Scenario: Recovery probe
- **WHEN** the cooldown has passed
- **THEN** exactly one request is sent to the endpoint as a probe; success restores it and failure reopens it with a longer cooldown

#### Scenario: Non-failover errors do not count
- **WHEN** an endpoint answers with a 4xx other than 408 and 429
- **THEN** its failure count is unchanged

### Requirement: Health never denies service
If every endpoint of a target is cooling down, the gateway SHALL still try them, those that reopen soonest first.

#### Scenario: All cold
- **WHEN** all endpoints of a target are open
- **THEN** the request is attempted against them rather than failing immediately

### Requirement: Health is observable
The gateway SHALL log each endpoint's transition between healthy and cooling down, and `GET /v1/health` SHALL report each endpoint's state, consecutive failures and remaining cooldown.

#### Scenario: Health report
- **WHEN** an authorized client calls `GET /v1/health`
- **THEN** it receives every configured endpoint with provider, model and state
