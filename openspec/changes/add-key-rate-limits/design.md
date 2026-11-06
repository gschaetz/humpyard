# Design

## Context

A request's pre-upstream checks run in a fixed order (`docs/invariants.md`, invariant 5): authenticate, check the key's route allow-list, check its budget, all before any upstream call, so a refusal costs no provider quota. Observed in the code: `authenticate` and `authorize` in `src/server.rs`, then `check_budget` before `plan_route`; refusals are `GatewayError` variants with a status and an `outcome` string, and `REFUSALS` in `src/metrics.rs` is the fixed vocabulary of `humpyard_refusals_total{reason}`. Keys are `[keys.<id>]` (`RawKey`) with budgets, a request cost ceiling and flags; there are no rate or concurrency settings.

One structural fact shapes the concurrency limit: the per-request `DecisionGuard` records the decision when it is dropped, and it is dropped when the handler returns, which for a streamed answer is when the headers are sent, not when the stream ends. A permit tied to the guard would be released while the stream is still running. The stream's own lifetime is the response body, so the permit has to ride on that.

In-force ADRs and invariants read: 0016 (metrics vocabulary is fixed), 0025 (keys, profiles, and the gate before sharing keys), 0017, invariant 5 (order of checks), invariant 7 (a request cancelled mid-flight is still recorded).

## Goals / Non-Goals

**Goals:**
- Contain a leaked, runaway or noisy key before it exhausts shared provider capacity, with a refusal that costs the providers nothing.
- A limit can never be spent by someone who is not the key's holder.
- A permit can never be lost: whatever ends a request, its permit is returned.

**Non-Goals:**
- Throttling failed authentication (see the proposal for why), limits shared across keys, per-route limits, token-rate limits, key profiles, limits shared across replicas.

## Decisions

**1. Per key, after authentication.** The limit is the authenticated key's, so only its holder can consume it: an attacker with a wrong key changes nothing, and nobody can lock a victim out by sending requests with the victim's id. This is why the limit is not per source address: behind a reverse proxy every request has the proxy's address, and an address-keyed limit applied before authentication would either be inert or lock out everyone.

**2. Position in the order.** Right after the route is authorised and before the budget check, so a rate-limited request never touches the budget. Invariant 5's list gains the limit.

**3. Exact sliding window for the rate.** For `max_requests_per_minute = N` the limiter keeps the admission times of the last N requests (a ring buffer of N timestamps) and admits a request only if the oldest is at least 60 seconds old. It is exact, easy to state and to test with an injected clock, and `Retry-After` is the remaining time of the oldest entry rounded up. Memory is N timestamps per limited key, bounded by the configuration maximum of 100000. A token bucket would allow bursts the spec does not describe.

**4. Concurrency by a permit on the response body.** Admission takes a permit (an atomic counter incremented against `max_concurrent`); the permit is a value whose `Drop` decrements it. A buffered response drops it when the response is sent; a streamed response carries it inside the body stream so it is dropped when the stream ends, errors or is dropped by the server because the client disconnected. Because release is `Drop`, every way a request can end (success, error, cancellation, a panic unwinding) returns the permit.

**5. Time source.** The limiter uses the gateway's `Clock`, as budgets do, so tests move time deterministically. Window arithmetic saturates and treats a clock that moves backwards as no time having passed, so a clock step cannot admit more than N or wedge the window.

**6. State.** One entry per configured key in a shared map, created from the configuration on load and on reload, with counters carried over for keys that remain and dropped for keys that do not. Nothing is keyed by anything a caller supplies, so the state cannot grow with traffic.

**7. The refusal.** A new refusal variant with status 429, `Retry-After`, and an OpenAI-style body (`error.type` `rate_limit_error`, `error.code` `rate_limit_exceeded`). It is recorded like other refusals (a decision with the outcome `rate_limited`) and added to `REFUSALS`, so `humpyard_refusals_total{reason="rate_limited"}` needs no new metric. It occurs before routing, so it cannot start failover and cannot touch endpoint health.

**8. Scope of counting.** Only endpoints that can reach a provider (chat completions, messages, responses) are counted; the read endpoints are not, so an operator can still read statistics while a key is limited.

## Threat model

Assets: the providers' shared rate limits and quota; the gateway's capacity to serve every client; the operator's spend velocity. Actors: the holder of a leaked key; a runaway agent loop; a noisy neighbour key; an unauthenticated caller; an operator who misconfigures a limit.

| # | Threat | Category | Mitigation | Proof |
|---|---|---|---|---|
| 1 | A leaked or runaway key spends a provider's per-minute quota in seconds, starving other clients (budgets bound spend, not speed) | Denial of service | `max_requests_per_minute` per key, refused before any upstream call | key-rate-limits spec; task 2.2 test |
| 2 | One key holds many slow streams open and monopolises in-flight capacity | Denial of service | `max_concurrent` with the permit held until the stream ends | task 2.3 test |
| 3 | A permit is never returned (a stream dropped mid-way, an error, a cancelled request) and the key is locked out | Denial of service | The permit is released by `Drop` on the response body | task 2.3 tests: finish, error, client disconnect |
| 4 | An unauthenticated caller spends or locks a victim key's allowance | Elevation, Denial of service | Limits are keyed by the authenticated key and checked after authentication; a failed authentication changes no counter | task 2.2 test |
| 5 | Limiter state grows without bound | Denial of service | One entry per configured key, nothing keyed by caller input; the window is at most 100000 timestamps | config validation; task 2.1 test |
| 6 | A clock step admits more than N or wedges a window | Tampering | Saturating arithmetic, a backwards step counts as no elapsed time | task 2.1 test |
| 7 | One actor with several keys multiplies its allowance | Elevation | Out of scope: keys are issued by the operator, and a shared limit comes with profiles (ADR 0025) | residual |
| 8 | Each replica enforces its own limit, so the effective limit is N times the replicas | Tampering | Documented; per-process by design | residual, documented |
| 9 | A limit set too low refuses a legitimate client | Denial of service | Limits are opt-in and per key; the refusal names the limit and gives `Retry-After`; the decision and the metric show it | spec; docs |
| 10 | Failed-authentication flooding or online guessing | Denial of service, Spoofing | Not addressed here: `keygen` keys are 256 random bits; documented that a weak operator-chosen secret is the operator's configuration error | proposal, residual |

Residual risks, accepted: per-process counters reset on restart; several replicas multiply the limit; a holder with several keys is not limited as one.

## Risks / Trade-offs

- **A permit rides the response body**, a new coupling between the request path and the body type. Contained to one wrapper, and covered by tests for every way a stream can end.
- **A limit refuses requests that would have succeeded.** That is the point; the message, `Retry-After` and the decision make the cause clear.
- **Exact windows use memory proportional to the limit** (8 bytes times N per key, at most about 800 KB for the maximum), acceptable and bounded.
- **A clock that jumps forward** frees a window early; a backwards jump never wedges it.

## Open Questions

None blocking. The maximum of 100000 requests per minute is chosen here; changing it before implementation is cheap.
