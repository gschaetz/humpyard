# route-selectors Specification

## Purpose
Defines how the gateway chooses the route for a request from facts about it (key, model, headers, tags, agent metadata), and that clients can narrow but never widen what a key may use.

## Requirements

### Requirement: Ordered selectors choose the route
The gateway SHALL evaluate `[[select]]` rules in order and route the request to the first matching rule whose route the key may use; when no rule applies it SHALL use the requested model as the route.

#### Scenario: Header selects a route
- **WHEN** a rule matches `header` `x-client` = `openclaw` and the request carries that header
- **THEN** the request is served by that rule's route and the response names the rule and route

#### Scenario: No rule matches
- **WHEN** no rule matches
- **THEN** the requested model is the route, and an unknown model is 404 as before

#### Scenario: First match wins
- **WHEN** two rules match
- **THEN** the earlier one decides

### Requirement: Conditions
A rule SHALL match only if every condition it gives holds: `model`, `key`, `profile`, `agent`, `task` (globs where `*` matches any text), `subagent`, `stream` (booleans), `header` and `tag` (names to globs). A rule without conditions matches every request.

#### Scenario: Tag condition
- **WHEN** a rule has `tag = { team = "infra" }` and the request has header `x-humpyard-tag-team: infra`
- **THEN** the rule matches

#### Scenario: Several conditions
- **WHEN** a rule requires a key glob and a subagent flag and only one holds
- **THEN** it does not match

### Requirement: Clients narrow, never widen
A selected route SHALL be one the key may use; a matching rule whose route the key may not use SHALL be skipped. Credential headers (`authorization`, `x-api-key`, `cookie`, `proxy-authorization`) SHALL never be matchable.

#### Scenario: Header cannot escalate a restricted key
- **WHEN** a key limited to route A sends a header that matches a rule for route B
- **THEN** the rule is skipped and the request follows the next applicable rule or the requested model

### Requirement: Selector config is validated
Loading SHALL fail naming the problem when a rule's route does not exist, a condition is unknown, a matched header is a credential header, rule names repeat, or a rule follows a catch-all rule.

#### Scenario: Unknown route
- **WHEN** a rule names a route that is not configured
- **THEN** config loading fails naming the rule and the route

### Requirement: Explain a routing decision without sending the request
The gateway SHALL answer `POST /v1/route/explain`, for the authenticated key, with the selected route and rule, the outcome each rule had (matched, the reasons it did not, and whether the key may use its route), the budget state, the targets and endpoint health the route would use, the route's fallback classes, and an outcome name. It SHALL use the same selection, budget and policy logic as real requests, SHALL NOT call any provider, and SHALL NOT write the ledger.

#### Scenario: Chosen rule and reasons
- **WHEN** a client asks to explain a request with a matching profile header
- **THEN** the report names the chosen rule and route and lists every other rule with why it did not match

#### Scenario: Skipped because of the key
- **WHEN** a rule matches but the key may not use its route
- **THEN** the report shows the rule as matched with `key_may_use_route` false, and the selected route comes from the next applicable rule or the requested model

#### Scenario: Refusals are named
- **WHEN** the request would be refused
- **THEN** the outcome is one of `forbidden`, `unknown_model`, `budget_exhausted` or `no_eligible_target`, with a message

#### Scenario: Agreement with real requests
- **WHEN** the same key, model and headers are sent as a real request
- **THEN** the real response's route and rule equal the explained ones

#### Scenario: Authentication required
- **WHEN** keys are configured and the call has none
- **THEN** it is refused with 401
