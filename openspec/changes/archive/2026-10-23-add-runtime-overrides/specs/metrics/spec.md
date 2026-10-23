# Spec Delta

## ADDED Requirements

### Requirement: Active overrides are counted with their age
The gateway SHALL export `humpyard_overrides_active` with the label `kind` and `humpyard_override_oldest_age_seconds`, read at scrape time, and the shipped alert rules SHALL include an example that fires when the oldest active override is more than six hours old.

#### Scenario: Counted by kind
- **WHEN** two `disable_target` overrides and one `pin_route` are active
- **THEN** the gauge shows 2 for `disable_target` and 1 for `pin_route`
