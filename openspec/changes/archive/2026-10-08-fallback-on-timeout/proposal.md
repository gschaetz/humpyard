# Proposal

## Why

A route lists targets in fallback order (for example a local model, then a hosted one). The 2026-10
fallback matrix test showed that Switchyard falls through to the next target on context-window
overflow, 429, 403, 408 and 5xx, but **not on a timeout**: a target that times out ends the
request with 504 while the next target is never tried. A hung local model is exactly when the
fallback is wanted, and LiteLLM-style gateways fall back on timeouts.

## What Changes

- When every endpoint of a target has failed and the last failure was a timeout, the pool
  re-labels it as a gateway-timeout HTTP error so Switchyard treats it as fallback-worthy. If no
  further target answers, the client still receives the same 504 `upstream timed out`.
- A regression test file pins the whole matrix of which failures fall back and which stop.

## Capabilities

### New Capabilities

### Modified Capabilities
- `provider-pool`: a timed-out target hands the request to the route's next target.

## Impact

`src/pool.rs`, `src/error.rs`, `tests/fallback.rs`, docs/routing.md.
