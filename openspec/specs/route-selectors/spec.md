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
