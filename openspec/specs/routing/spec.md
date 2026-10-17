# routing Specification

## Purpose
TBD - created by archiving change add-tiered-fallback. Update Purpose after archive.

## Requirements

### Requirement: Guarded tail on a pool route
A `fastest` route with a `pool` SHALL accept `then = { targets, on }`. The planned candidates SHALL be the pool members in their usual order followed by the `then` targets in written order. A failed attempt SHALL move to a `then` target only when its failure class is in `then.on` (default `overflow`); failover among pool members SHALL be unaffected. Configuration SHALL be refused when a `then` target does not exist, is also a pool member, or has a class the route's `allowed_classes` excludes.

#### Scenario: A prompt larger than every free model
- **WHEN** a request's estimated prompt exceeds every pool member's declared window
- **THEN** the pool members are subtracted by planning and the first `then` target serves the request

#### Scenario: A free model fails
- **WHEN** a pool member answers with a transport error or 429
- **THEN** the next pool member is tried, and the tail is reached only if every member is exhausted and the class is in `then.on`

#### Scenario: A failure class not in the guard
- **WHEN** every pool member fails with a class not listed in `then.on`
- **THEN** the request fails with that error and no `then` target is called

#### Scenario: Invalid tail
- **WHEN** `then.targets` names an unknown target, a pool member, or a target whose class the route forbids
- **THEN** the configuration is refused naming the route and target

#### Scenario: Explain
- **WHEN** a route with a tail is explained
- **THEN** the report lists the tail targets and their guard classes
