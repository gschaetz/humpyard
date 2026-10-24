# budget-enforcement Specification

## Purpose
Defines per-key budgets and what the gateway does as a key approaches and exceeds them.

## Requirements

### Requirement: Budget limits and periods
A key MAY declare daily and monthly limits in USD and in tokens; periods SHALL be UTC calendar days and months.

#### Scenario: New day
- **WHEN** the UTC date changes
- **THEN** the key's daily spend starts again from zero while its monthly spend continues

### Requirement: Live spend
A key's counters SHALL include a request's usage as soon as that request finishes, so the next request sees it.

#### Scenario: Back-to-back requests
- **WHEN** one request completes and costs $0.50
- **THEN** a following request from the same key sees $0.50 more spend

### Requirement: Exhausted budget blocks by default
When any limit of a key is reached the gateway SHALL answer with HTTP 402 in the endpoint's error format before making any upstream call, unless the key's `over_budget` is `free_only`.

#### Scenario: Daily limit reached
- **WHEN** a key at its daily USD limit sends a request
- **THEN** the response is 402 naming the limit and no upstream call is made

### Requirement: Free-only mode
A key with `over_budget = "free_only"` SHALL continue to be served from zero-priced targets only once a limit is reached, and SHALL receive 402 if no zero-priced target is eligible.

#### Scenario: Over budget with a free target
- **WHEN** such a key is over budget and the route has a zero-priced target
- **THEN** the request is served by that target

### Requirement: Restricted state
When a key's spend reaches the configured `restricted_at` fraction (default 0.8) of any limit, the gateway SHALL make targets whose output price exceeds the configured restricted price ceiling ineligible for that key, and tier substitution SHALL apply as for any policy exclusion.

#### Scenario: Premium target excluded near the limit
- **WHEN** a key is at 85% of its daily limit and `smart` exceeds the ceiling
- **THEN** a request that would escalate to `smart` is served by an eligible cheaper target

### Requirement: Bounded overshoot
Because spend is known only when a request finishes, requests already in flight MAY take a key past a limit, and the gateway SHALL NOT start new requests after the limit is observed.

#### Scenario: Concurrent requests at the edge
- **WHEN** several requests start just under the limit and finish above it
- **THEN** later requests are refused with 402

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
