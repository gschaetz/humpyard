## ADDED Requirements

### Requirement: Unavailable endpoints are counted
The gateway SHALL count every provider attempt whose failure class is `unavailable` in `humpyard_endpoint_unavailable_total` with the labels `provider` and `target`, both from the bounded vocabulary of configured names (a name not in the configuration SHALL appear as `other`). The counter SHALL carry no model name, error text or key, and SHALL be counted once per failed attempt.

#### Scenario: A retired model
- **WHEN** a provider answers `410` for a target's model and the request fails over to another target
- **THEN** `humpyard_endpoint_unavailable_total{provider="opencode",target="pf3"}` increases by one

#### Scenario: A failure of another class
- **WHEN** an attempt fails with a rate limit, a timeout or a server error
- **THEN** the counter does not change

#### Scenario: Hostile names
- **WHEN** an attempt names a target or provider that the configuration does not define
- **THEN** the series uses `other` for it and no client string reaches the output
