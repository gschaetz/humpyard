# Spec Delta

## ADDED Requirements

### Requirement: An endpoint may declare a per-request cap
An endpoint MAY declare `max_request_tokens`, a whole number of at least 1: the most prompt tokens one request may carry to that provider. It is separate from `context_window` and SHALL NOT change a pool's `min_context_window` membership. An endpoint that declares no cap SHALL have none.

#### Scenario: A cap beside a large window
- **WHEN** an endpoint declares `context_window = 131072` and `max_request_tokens = 8000`
- **THEN** the configuration loads and a pool with `min_context_window = 32000` still includes the target

#### Scenario: No cap
- **WHEN** an endpoint declares no `max_request_tokens`
- **THEN** it has no cap and the endpoint behaves exactly as before

### Requirement: A per-request cap is validated
A `max_request_tokens` of zero, negative, not a whole number, or greater than the endpoint's declared `context_window` SHALL be refused at load, naming the target, the endpoint and the setting.

#### Scenario: Zero
- **WHEN** an endpoint declares `max_request_tokens = 0`
- **THEN** the configuration is rejected naming the target, the endpoint and `max_request_tokens`

#### Scenario: Larger than the window
- **WHEN** an endpoint declares `context_window = 8000` and `max_request_tokens = 131072`
- **THEN** the configuration is rejected naming the target, the endpoint and `max_request_tokens`
