# Proposal

## Why

The gateway forwards to a single upstream with no decisions. The project's purpose is to pick the
right model per request (cheap first, escalate on trouble) and survive provider failures. Switchyard
0.3.0 supplies the routing algorithms; we supply providers, failover and the seam where budget and
provider-health awareness plug in later.

## What Changes

- **BREAKING** Replace the single `[upstream]` config with `[[providers]]`, `[[targets]]` and
  `[[routes]]`. Pre-alpha, so no migration path from the old shape.
- Run each request through a Switchyard algorithm chosen by the requested route. Built-ins only:
  `passthrough`, `random`, `stage_router`, and `llm_classifier`. The algorithms see the request
  and session, and select a target plus ordered fallbacks.
- Resolve a target to ordered provider endpoints and fail over across them on connection errors,
  timeouts, 5xx and 429.
- Add a routing-policy seam: before each run, a policy narrows which targets are eligible. The
  shipped policy allows every target. Budget and health policies are separate later changes.
- Report which target and provider served a request via response headers and logs.

Out of scope: budget/cost tracking, active health checks, custom (non-built-in) algorithms,
Switchyard's advisor-gate, plan-execute, composite and subagent algorithms, the modelrelay
migration command, inbound auth.

## Capabilities

### New Capabilities
- `provider-pool`: providers, targets with ordered endpoints, and failover between endpoints.
- `request-routing`: routes backed by built-in Switchyard algorithms, session continuity, and serving attribution.
- `routing-policy`: the per-request eligibility hook that narrows targets before routing.

### Modified Capabilities
- `gateway-config`: multi-provider configuration replaces the single upstream.
- `upstream-proxy`: forwarding and error mapping apply per provider endpoint, with failover.
- `gateway-api`: model listing returns routes and targets.

## Impact

- Adds `switchyard-libsy` and `switchyard-llm-client` 0.3.0; existing `switchyard-protocol` and
  `-translation` stay.
- `src/server.rs` handler is restructured around a routing step; `src/config.rs` schema changes.
- Streaming path gains a pre-dispatch routing phase; classifier calls add latency for `llm_classifier` routes only.
