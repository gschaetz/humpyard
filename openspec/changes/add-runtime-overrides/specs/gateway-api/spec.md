# Spec Delta

## ADDED Requirements

### Requirement: Overrides are created, listed and removed through the API
The gateway SHALL provide `POST /v1/overrides`, `GET /v1/overrides` (with `?history=true`, which also lists the overrides of the last 30 days that have ended, each with its status and when it ended) and `DELETE /v1/overrides/{id}`. A create SHALL name a kind (`disable_target`, `drain_provider` or `pin_route`), its subject, a reason and optionally `expires_in_secs`; it SHALL return the override with its id and expiry. A removal SHALL take effect for the next request.

#### Scenario: Create and list
- **WHEN** a key with `manage_overrides` creates a `disable_target` for a target and then lists overrides
- **THEN** the override appears with its kind, subject, reason, creator id and expiry

#### Scenario: Remove
- **WHEN** an override is removed
- **THEN** the next request is planned without it and the removal is in the audit trail

### Requirement: Writing overrides needs an opt-in key
Creating or removing an override SHALL require a key with `manage_overrides = true`; any other key SHALL receive HTTP 403 with the error code `overrides_not_manageable`. Listing SHALL require `manage_overrides` or `read_config`. A gateway with no keys configured SHALL allow all of it.

#### Scenario: An ordinary key
- **WHEN** a key without the setting tries to create an override
- **THEN** the answer is 403 `overrides_not_manageable` and nothing is stored

### Requirement: Overrides must be bounded and must name existing things
A create without a ledger SHALL be refused with 409. An expiry SHALL be required (a default applies when omitted) and SHALL NOT exceed the configured maximum. A subject that is not in the loaded configuration SHALL be refused, and a `pin_route` to a target that is not a candidate of that route, or whose class a strict route forbids, SHALL be refused.

#### Scenario: Too long
- **WHEN** an override asks for an expiry beyond the maximum
- **THEN** it is refused and nothing is stored

#### Scenario: Unknown subject
- **WHEN** an override names a target that does not exist
- **THEN** it is refused

#### Scenario: A pin cannot cross a class
- **WHEN** a private-only route is pinned to a public target
- **THEN** it is refused
