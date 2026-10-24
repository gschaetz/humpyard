# Spec Delta

## ADDED Requirements

### Requirement: A key may set a per-request cost ceiling
A key MAY set `max_request_usd`, a positive number of US dollars, and a configuration with such a key SHALL require every endpoint to carry a price, as it does for a USD budget. A non-positive value SHALL be rejected when the configuration loads.

#### Scenario: A valid ceiling
- **WHEN** a key sets `max_request_usd = 0.50` and every endpoint is priced
- **THEN** the configuration loads

#### Scenario: An unpriced endpoint
- **WHEN** a key sets `max_request_usd` and an endpoint has no price
- **THEN** the configuration is refused naming the endpoint

#### Scenario: A bad value
- **WHEN** a key sets `max_request_usd = 0` or a negative number
- **THEN** the configuration is refused

### Requirement: Targets certain to exceed the ceiling are removed
For a key with a ceiling, a target SHALL be removed from a request's candidates only when a lower bound of the request's cost on that target exceeds the ceiling. The lower bound SHALL use the low prompt estimate, the output length the client requested (zero when none), and the lowest input and output rates among the target's endpoints and tiers. A target whose endpoints are all priced zero SHALL never be removed by the ceiling. A key without a ceiling SHALL be unaffected.

#### Scenario: A large prompt removes the dear target and keeps the free one
- **WHEN** a request's prompt alone costs more than the ceiling on a paid target and the route also has a free target
- **THEN** the paid target is removed and the free target serves the request

#### Scenario: Unknown output is not assumed
- **WHEN** the client sets no output limit and the prompt alone costs less than the ceiling
- **THEN** the target is kept

#### Scenario: A cheap failover endpoint keeps the target
- **WHEN** a target's cheapest endpoint would cost less than the ceiling and another endpoint would cost more
- **THEN** the target is kept

#### Scenario: No ceiling, no change
- **WHEN** a key sets no ceiling
- **THEN** routing is exactly as before

### Requirement: A request with no remaining target is refused as too costly
When the ceiling leaves no candidate, the gateway SHALL refuse the request before any upstream call with HTTP 402 and the error code `request_cost_ceiling`, whose message states the ceiling and the lower-bound estimate for the cheapest candidate, distinct from `budget_exhausted`.

#### Scenario: Everything is too dear
- **WHEN** every candidate on the route is paid and a request's lower-bound cost exceeds the ceiling on all of them
- **THEN** the response is 402 `request_cost_ceiling` and no upstream is called

#### Scenario: Distinct from an exhausted budget
- **WHEN** a key with room in its daily budget sends such a request
- **THEN** the error code is `request_cost_ceiling`, not `budget_exhausted`

### Requirement: The ceiling is visible
A decision SHALL record the ceiling and the targets it removed, `explain` SHALL list them with the lower bound, and the gateway SHALL count requests where the ceiling removed targets or refused in `humpyard_cost_ceiling_total` with an `outcome` label.

#### Scenario: Explain shows the removal
- **WHEN** a route is explained for a key and a prompt size that the ceiling affects
- **THEN** the report lists the removed targets, the ceiling and the lower bound

#### Scenario: The metric counts it
- **WHEN** the ceiling removes a target for a request
- **THEN** `humpyard_cost_ceiling_total{outcome="removed"}` increases by one
