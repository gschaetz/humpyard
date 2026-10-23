# Spec Delta

## ADDED Requirements

### Requirement: Active overrides only remove candidates
Active overrides SHALL be applied in planning after class strictness and capabilities and before health, and SHALL only remove candidates: `disable_target` removes a target, `drain_provider` removes every endpoint of a provider, and `pin_route` cuts a route's candidates to the pinned target. An override SHALL stop having any effect at its expiry without any other action.

#### Scenario: A disabled target is skipped
- **WHEN** a target is disabled and a route would otherwise try it
- **THEN** the request is served by another candidate and the disabled target is not called

#### Scenario: An override expires
- **WHEN** a target is disabled for one hour and an hour passes
- **THEN** the target is eligible again with no other action

#### Scenario: A drained provider
- **WHEN** a provider is drained
- **THEN** none of its endpoints is called, and a target with endpoints on other providers still serves through those

#### Scenario: A pin
- **WHEN** a route is pinned to one of its targets
- **THEN** only that target serves the route until the pin ends

### Requirement: An override that removes every candidate refuses the request
When overrides remove every candidate, the gateway SHALL refuse the request with HTTP 503 and the error code `override_no_target`, before any upstream call, and the decision SHALL record the overrides that applied.

#### Scenario: The last target is disabled
- **WHEN** the only eligible target of a route is disabled
- **THEN** the request is refused with `override_no_target` and removing the override restores service at once
