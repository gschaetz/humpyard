# metrics Specification

## Purpose
TBD - created by archiving change add-health-eligibility. Update Purpose after archive.

## Requirements

### Requirement: Health skips are counted
The gateway SHALL count, per route and target, requests in which a target was skipped because it was cold, as `humpyard_health_skips_total{route,target}`. Both labels SHALL be configured names.

#### Scenario: A cold target is skipped
- **WHEN** a request on a route skips a cold target
- **THEN** the counter for that route and target increases by one

#### Scenario: Nothing skipped
- **WHEN** no target is cold, or every eligible target is cold (fail open)
- **THEN** the counter does not change

### Requirement: Capability removals are counted
The gateway SHALL count, per route, target and reason, requests in which a target was removed for capability, as `humpyard_capability_skips_total{route,target,reason}`. `route` and `target` SHALL be configured names and `reason` one of `tools`, `vision` or `context`.

#### Scenario: An oversized prompt
- **WHEN** a target is removed because the prompt exceeds its window
- **THEN** the counter for that route, target and `context` increases by one

### Requirement: Cache tokens are counted
`humpyard_tokens_total` SHALL also carry the `direction` values `cached_input` and `cache_creation`, so the cache-hit ratio can be derived in a dashboard. Session ids SHALL NOT be used as a metric label.

#### Scenario: A cached call
- **WHEN** a call reports cached input tokens
- **THEN** the `cached_input` series for its route and target increases by that number

### Requirement: Blocked fallbacks are counted
The gateway SHALL count, per route, requests that ended in an upstream failure on a route whose `allowed_classes` excluded one or more targets, as `humpyard_fallback_blocked_total{route}`. The label SHALL be a configured route name.

#### Scenario: Strict route fails
- **WHEN** a request fails on a strict route that excluded a public target
- **THEN** `humpyard_fallback_blocked_total` for that route increases by one

#### Scenario: Strict route succeeds or route is not strict
- **WHEN** a request on a strict route succeeds, or fails on a route with no exclusions
- **THEN** the counter does not change

### Requirement: Metrics are served only when enabled
The gateway SHALL serve `GET /metrics` only when `[metrics] enabled = true`. Otherwise the path SHALL behave as any unknown path.

#### Scenario: Disabled by default
- **WHEN** the gateway runs with no `[metrics]` block
- **THEN** `GET /metrics` returns 404

### Requirement: Metrics have their own credential
On the main listener, `/metrics` SHALL require the configured metrics credential and SHALL refuse a virtual key. The metrics credential SHALL NOT authorize any other endpoint. Enabling metrics on the main listener without a credential SHALL fail validation.

#### Scenario: Client key is refused
- **WHEN** a request to `/metrics` carries a valid virtual key but not the metrics credential
- **THEN** the response is 401

#### Scenario: Metrics credential cannot call inference
- **WHEN** a request to `/v1/chat/completions` carries the metrics credential
- **THEN** the response is 401

### Requirement: A separate listener needs no credential
When `[metrics] listen` is set, `/metrics` SHALL be served only on that address and SHALL NOT be served on the main listener.

#### Scenario: Not on the public port
- **WHEN** `listen` is set and `/metrics` is requested on the main listener
- **THEN** the response is 404

### Requirement: Labels come only from configuration or fixed sets
Every metric label value SHALL be a name from the loaded configuration or a member of a fixed set. A string supplied by a client (requested model, header, tag, prompt) SHALL NOT become a label value.

#### Scenario: Unknown model name
- **WHEN** a client requests a model name no rule matches
- **THEN** the request is counted under `route="none"` and the requested name appears in no series

### Requirement: Metrics never carry identity or content
By default no series SHALL carry a key id, key hash, tenant or request content. With `key_labels = true`, a `key` label (the configured key id) SHALL appear on `requests_total`, `tokens_total`, `cost_micro_usd_total` and `refusals_total` only, never on a histogram, and never with a hash or secret.

#### Scenario: Key labels are off
- **WHEN** a keyed request completes
- **THEN** no exposed series contains the key id

#### Scenario: Key labels are on
- **WHEN** `key_labels = true` and a keyed request completes
- **THEN** the four counters carry the key id and no histogram does

### Requirement: Key labels are bounded
The number of distinct `key` label values SHALL NOT exceed `key_label_limit`; keys past the limit SHALL be counted under `key="other"`.

#### Scenario: Cap reached
- **WHEN** more distinct keys than `key_label_limit` make requests
- **THEN** the extra keys are summed under `key="other"` and the series count stays bounded

### Requirement: Exposition is valid and monotonic
The output SHALL be valid Prometheus text format with HELP and TYPE lines, counters SHALL never decrease across scrapes within a process, and histogram buckets SHALL be cumulative.

#### Scenario: Strict parse
- **WHEN** the output is parsed by a strict text-format reader
- **THEN** it parses with no error and every histogram's buckets are non-decreasing

### Requirement: Metrics never slow or fail a request
Updating the registry SHALL be non-blocking and SHALL NOT fail or delay a request. A scrape SHALL NOT take a lock that a request path holds.

#### Scenario: Concurrent scrape
- **WHEN** requests are served while `/metrics` is scraped repeatedly
- **THEN** no request fails or is measurably delayed by the scrape

### Requirement: Registry is shaped for other views
The registry SHALL expose a read-only snapshot of every metric with its name, type, help text and label sets, from which the text encoder is built, so that another view can be added without a second source of numbers.

#### Scenario: Snapshot matches exposition
- **WHEN** the snapshot is rendered as text
- **THEN** it equals the `/metrics` body for the same instant

### Requirement: The configuration fingerprint is a metric
The gateway SHALL export `humpyard_config_info` with the label `fingerprint` set to the fingerprint of the loaded configuration and the value 1. After a reload it SHALL export only the new fingerprint, so one series exists at a time.

#### Scenario: A reload replaces the series
- **WHEN** the configuration is reloaded and its fingerprint changes
- **THEN** the metric shows the new fingerprint and no series for the old one

### Requirement: Active overrides are counted with their age
The gateway SHALL export `humpyard_overrides_active` with the label `kind` and `humpyard_override_oldest_age_seconds`, read at scrape time, and the shipped alert rules SHALL include an example that fires when the oldest active override is more than six hours old.

#### Scenario: Counted by kind
- **WHEN** two `disable_target` overrides and one `pin_route` are active
- **THEN** the gauge shows 2 for `disable_target` and 1 for `pin_route`
