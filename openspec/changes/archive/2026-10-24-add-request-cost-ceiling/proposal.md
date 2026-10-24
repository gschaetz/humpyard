# Proposal

## Why

Budgets today are about totals: a key may spend so much per day or month, and a request is refused only once that
total has been reached. Cost is known only when a call finishes, so a single very large request, a runaway agent
loop or one oversized prompt can take a key far past its limit before the limit is observed ("overshoot is bounded,
not zero"). Operators who hand a key to an agent or to another person want a second, simpler guarantee that does
not depend on totals: **no single request may cost more than X**. This is what people ask of a gateway when they
share access, and it is independent of whether a daily budget is configured.

**Status: designed, deliberately not built.** The operator's own paid provider (OpenCode Go) is a flat-rate plan whose
limit is a quota, so no dollar control applies to it. This is kept ready for the first pay-as-you-go provider, or
for keys shared with others; see the roadmap.

## What Changes

- A key MAY set `max_request_usd`, a ceiling on what one request may cost.
- Before routing, any target for which the request is **certain** to cost more than the ceiling is removed,
  using the same "remove only when certain" rule as context windows (ADR 0019): a deliberately low prompt
  estimate, the lowest rate of the target's endpoints, and the output length the client asked for.
- Free targets always remain eligible, so a request that is too dear for the paid targets can still be served by a
  free one on the same route (the existing `free_only` behavior, per request).
- When no target remains, the request is refused with HTTP 402 and a distinct code, before any upstream call.
- The decision, `explain` and a metric say which targets the ceiling removed.
- Out of scope: truncating or rewriting the request, a ceiling per route or per target, and the profile field
  (ADR 0025), which will carry the same setting when profiles exist.

## Capabilities

### Modified Capabilities
- `budget-enforcement`: a per-request cost ceiling on a key.

## Impact

Configuration (`max_request_usd` on a key, validated like a USD budget: every endpoint priced), the budget policy
and planning, the decision log, `explain` and one metric. No change for keys without the setting.
