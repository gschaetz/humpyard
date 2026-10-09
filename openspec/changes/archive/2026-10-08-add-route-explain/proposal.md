# Proposal

## Why

Selector rules are order-sensitive and depend on keys, headers and budgets, so operators (and
customers writing their own rules) cannot tell what a request would do without sending one and
spending tokens. Shadowed or skipped rules are especially hard to see. A dry run closes the gap.

## What Changes

- `POST /v1/route/explain` takes `{ "model", "stream", "headers", "agent", "task", "subagent" }`
  (only `model` required) and answers, for the calling key: the selected route, rule and whether
  it came from a selector or the requested model; every rule with whether it matched, why not, and
  whether the key may use its route; the budget state; the targets the route would use after the
  routing policy, each endpoint's health, and the route's `fallback_on`; and an outcome
  (`ok`, `forbidden`, `unknown_model`, `budget_exhausted`, `no_eligible_target`).
- It runs the same selection, budget and policy code as real requests, never calls a provider and
  never writes the ledger. Credential headers in the hypothetical request are ignored.
- README and docs updated (status, endpoints, selectors, fallback_on, health).

Out of scope: explaining as another key (needs the admin credential), request-feature conditions.

## Capabilities

### New Capabilities

### Modified Capabilities
- `route-selectors`: adds the dry-run explanation.

## Impact

`src/select.rs` (rule trace), `src/server/explain.rs` (new), routes, tests, docs, README.
