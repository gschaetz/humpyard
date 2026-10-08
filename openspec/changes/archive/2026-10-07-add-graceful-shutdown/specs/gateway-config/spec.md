# Spec Delta

## ADDED Requirements

### Requirement: Shutdown grace period
The configuration SHALL accept an optional top-level `shutdown_grace_secs`, a whole number of seconds from 0 to 3600 with a default of 30, bounding how long the gateway waits for in-flight requests after a termination signal.

#### Scenario: Default
- **WHEN** the key is absent
- **THEN** the grace period is 30 seconds

#### Scenario: Out of range
- **WHEN** the value is above 3600
- **THEN** the process exits non-zero naming the key
