# Proposal

## Why

Moving the cluster's public tiers off ModelRelay needs a route that means "the free pool, and only when a
prompt is too large for every free model, a paid target". A pool route cannot name extra fixed targets, and
`fallback_on = ["overflow"]` on a plain list route would also stop one free model failing over to another.
Without this the swap loses the paid overflow, and an oversized prompt fails instead of being served.

## What Changes

- A `fastest` route with a `pool` MAY have `then = { targets = [...], on = [...] }`: a tail of fixed targets
  appended after the pool members, reached only on the listed failure classes (the `fallback_on` names). Pool
  members still fail over among themselves on the route's own rules.
- `then.on` defaults to `["overflow"]`. Tail targets are ordinary targets: strict classes, capabilities and
  health apply to them as to any candidate.
- Out of scope: tails on routes other than pool `fastest`, more than one tail, ordering inside the tail
  (written order).

## Capabilities

### Modified Capabilities
- `routing`: pool routes gain an optional guarded tail.

## Impact

`src/config/schema.rs` and validation (new field), `src/routing.rs` planning (append the tail with its guard),
`src/pool.rs` (fall to the tail only on its classes), `explain` (shows the tail and why it was or was not
reached), docs and a new ADR. No change to routes without `then`.
