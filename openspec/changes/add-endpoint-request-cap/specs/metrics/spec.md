# Spec Delta

## MODIFIED Requirements

### Requirement: Capability removals are counted
The gateway SHALL count, per route, target and reason, requests in which a target was removed for capability, as `humpyard_capability_skips_total{route,target,reason}`. `route` and `target` SHALL be configured names and `reason` one of `tools`, `vision`, `context` or `request_cap`.

#### Scenario: An oversized prompt
- **WHEN** a target is removed because the prompt exceeds its window
- **THEN** the counter for that route, target and `context` increases by one

#### Scenario: A prompt over a per-request cap
- **WHEN** a target is removed because the prompt exceeds its `max_request_tokens`
- **THEN** the counter for that route, target and `request_cap` increases by one
