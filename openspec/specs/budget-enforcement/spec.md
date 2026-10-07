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
