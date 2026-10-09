# Proposal

## Why

Today the client's `model` field names the route, so every client of a key gets the same flow
and routing cannot depend on who is asking or what they say about the request. Operators
(and eventually customers of a hosted deployment) need several rule sets side by side, with the
one a request follows chosen from request facts: the key, the requested model, headers such as a
profile or tags sent by a client like openclaw, and the agent metadata Switchyard already
extracts (agent id, subagent flag, task id).

## What Changes

- New ordered `[[select]]` rules. Each has an optional `name`, a `when` condition (all given
  conditions must hold) and a `route`. The first rule that matches **and names a route the key may
  use** decides the route; if none does, the requested model is the route, exactly as today.
- Conditions: `model`, `key` (globs), `profile`, `agent`, `task` (globs), `subagent`, `stream`
  (booleans), `header` (name to glob), `tag` (name to glob, sugar for `x-humpyard-tag-<name>`).
- Standard client headers: `x-humpyard-profile` (a free-form profile name) and
  `x-humpyard-tag-<name>` (free-form tags). Any other header can be matched by naming it
  explicitly; credential headers can never be matched.
- Clients can narrow but never widen: a selected route must pass the key's `allowed_routes`,
  otherwise that rule is skipped.
- Responses carry `x-humpyard-route` and `x-humpyard-rule` so the decision is visible; the
  ledger's route column records the route that served.
- Config validation: selector routes must exist, unknown condition names and credential headers
  are rejected, rules after a catch-all are rejected as unreachable.

Out of scope (later steps): request-feature conditions (prompt size, tools), an `explain` call,
hot reload, expression languages, per-hop conditions inside a route.

## Capabilities

### New Capabilities
- `route-selectors`: choosing the route from request facts, safely.

### Modified Capabilities

## Impact

New core module `src/select.rs`; `src/config/*` (parsing, validation), `src/server.rs`
(authorize stage), tests, docs, ADR 0011.
