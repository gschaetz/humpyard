# Proposal

## Why

Failover only helps after a failure. With a dead or rate-limited provider first in a target, every
request pays its full timeout before the next endpoint runs: a black-holed first endpoint added
3.6 s to every request in the 2026-10-08 real-network test, and nothing remembers it just failed.
Surviving flaky free providers is the reason to run a pool at all.

## What Changes

- Each endpoint (provider + model) gets an in-memory circuit breaker. After
  `failure_threshold` consecutive failover-class failures (connect, timeout, 408, 429, 5xx) it is
  skipped for `cooldown_secs`, doubling on each repeated trip up to `max_cooldown_secs`. When the
  cooldown ends one request is let through as a probe: success closes the breaker, failure
  reopens it with a longer cooldown.
- **Fail open:** if every endpoint of a target is cooling down, the target is still tried (the
  ones due to reopen soonest first), so health can slow nothing down and never deny service.
- New optional `[health]` config section; `failure_threshold = 0` turns it off.
- `GET /v1/health` lists each endpoint's state (authenticated like the other `/v1` reads).
- State changes are logged.

Out of scope (follow-up): removing a fully cold target from the routing policy so tier
substitution applies; Retry-After handling; persisting state across restarts; counting failures
that happen mid-stream.

## Capabilities

### New Capabilities

### Modified Capabilities
- `provider-pool`: endpoint circuit breaking and health reporting.
- `gateway-config`: the `[health]` section.

## Impact

New module `src/health.rs`, changes to `src/pool.rs`, `src/config/*`, `src/server.rs`, the
architecture rules, tests, docs, ADR 0009.
