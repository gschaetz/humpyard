# Spec Delta

## ADDED Requirements

### Requirement: Health configuration
The config MAY contain a `[health]` section with `failure_threshold` (default 3), `cooldown_secs` (default 30) and `max_cooldown_secs` (default 300). A threshold of 0 SHALL disable endpoint health tracking. Invalid values (cooldown 0 while enabled, max below cooldown, values above one day) SHALL be rejected naming the key.

#### Scenario: Defaults
- **WHEN** the section is absent
- **THEN** health tracking is on with 3, 30 and 300

#### Scenario: Disabled
- **WHEN** `failure_threshold = 0`
- **THEN** no endpoint is ever skipped

#### Scenario: Invalid
- **WHEN** `max_cooldown_secs` is below `cooldown_secs`
- **THEN** loading fails naming `max_cooldown_secs`
