# Proposal

## Why

Routes fall through to their next target on every failover-class error. When the next tier is
paid, a transient blip in a free tier spends paid credit, and operators cannot express "only go to
the expensive tier when the prompt does not fit". This is step 1 of making routing rules
customer-definable (ladders with per-step conditions, then selectors on request metadata).

## What Changes

- Every route type accepts `fallback_on`, a list of failure classes that may hand the request to
  the next target: `overflow`, `rate_limit`, `timeout`, `server_error`, `connection`, `forbidden`.
  Absent means all of them (unchanged behavior). `[]` disables target fallback.
- A failure outside the list ends the request and the client receives the original status and
  message.
- Unknown class names are rejected when the config loads.

Out of scope: per-step (per-hop) conditions inside one route, conditions on request features,
selectors (later steps of the same direction); endpoint rotation inside a target is unchanged.

## Capabilities

### New Capabilities

### Modified Capabilities
- `request-routing`: routes can restrict which failures fall through to the next target.

## Impact

`src/config/schema.rs` (type and field), `src/error.rs` (classification, stop marker),
`src/metering.rs` (applied at the metered client), `src/routing.rs`, `src/server.rs`, tests, docs.
