# Proposal

## Why

A key can be limited by what it may spend (daily and monthly budgets, a ceiling per request) and by which routes it may use, but not by how fast it may call or how many requests it may hold open. The threat model (`docs/threat-model.md`, the gap on rate limits) names why that matters: several clients share the gateway's provider keys, and provider rate limits and quotas are shared with them. A leaked key, a runaway agent loop or a noisy neighbour can spend a provider's per-minute quota in seconds, or hold dozens of long streams open, and every other client then sees 429s from the providers or waits, long before any budget is reached. ADR 0025 already makes this a precondition for sharing keys with anyone but the operator.

## What Changes

- A key gains two optional limits, both off unless set: `max_requests_per_minute` (an exact count over any 60 seconds) and `max_concurrent` (requests in flight at once, where a streamed answer counts until it ends or the client goes).
- A request over a limit is refused at the front door with **429**, an OpenAI-style error body and a `Retry-After` header, before any upstream call, so it costs no provider quota and never triggers failover.
- The check runs after the key is authenticated and its route is allowed and before the budget check, so an unauthenticated caller can never spend another key's allowance.
- A refusal is visible like the others: the decision's outcome is `rate_limited`, `humpyard_refusals_total{reason="rate_limited"}` counts it, and the effective configuration shows the limits.
- State is in memory, per gateway process. A restart clears it, and each replica enforces its own limit; the documentation says so.
- **Not in scope, and why.** Throttling failed authentication is deliberately left out. Keys from `keygen` are 256 random bits, so guessing one online is not a realistic attack; a per-source throttle behind a reverse proxy sees only the proxy's address, so it either does nothing or locks out everyone. A weak, operator-chosen secret is a configuration error the documentation addresses (use `keygen`). Also out of scope: a limit shared by several keys, per-route limits, tokens-per-minute limits, and limits in key profiles (ADR 0025), which will reuse this mechanism.

## Capabilities

### New Capabilities

- `key-rate-limits`: the request-rate and concurrency limits on a key, where they are applied, how a refusal looks, how long a permit is held, and how limits follow a reload.

### Modified Capabilities

- `gateway-config`: the two key fields and their validation.

## Impact

- `src/config/` (the key fields), a new limiter module (pure, with an injectable clock), and the request path in `src/server.rs` (an in-flight permit that rides the response body), `src/error.rs` (a refusal with status 429 and `Retry-After`), `src/metrics.rs` (the new refusal reason).
- Docs: `configuration.md`, `api.md` (the 429), `deployment.md` (one limiter per replica), `invariants.md` (the order of checks), `threat-model.md` (the gap closed), `architecture.md`.
- No change to routing, failover, budgets, the ledger schema or the admin API's contract. Existing configurations are unaffected: no key has a limit until one is set.
