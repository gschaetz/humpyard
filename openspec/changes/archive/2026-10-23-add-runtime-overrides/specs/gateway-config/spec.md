# Spec Delta

## ADDED Requirements

### Requirement: Override settings and the manage_overrides key setting
The configuration MAY set `[overrides] default_secs` (default 14400) and `max_secs` (default 86400, at most 604800), with the default not above the maximum, and a key MAY set `manage_overrides = true` (default false). Invalid values SHALL be rejected when the configuration loads.

#### Scenario: A default above the maximum
- **WHEN** `default_secs` is greater than `max_secs`
- **THEN** the configuration is refused
