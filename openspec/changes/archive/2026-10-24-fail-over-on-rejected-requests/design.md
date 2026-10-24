# Design

## Context

Fallback is decided per error by `fallback_class` (src/error.rs) and applied by `Metered` (src/metering.rs): a class a
route does not allow is stopped (`stop_fallback`); `unavailable` and a 413 are re-labelled as a 503 because
Switchyard only moves on for a 503, and the original answer is restored if no target takes the request. `FallbackOn`
(src/config/schema.rs) holds the allowed classes and `unavailable` is on by default only for `fastest` and pool
routes. `CallContext.attempts` already records every attempt of the request with its provider and outcome.

## Decisions

- **A new class, not a widening of `unavailable`.** `rejected` differs in the way that matters: it says nothing about
  the endpoint's health, so it must not count against the breaker (as a 413 does not) while `unavailable` must.
  Keeping them apart also keeps ADR 0023's tested set of endpoint-describing statuses intact.
- **Default only where the route is built to try many targets.** Same rule as `unavailable`: `fastest` and pool
  routes. A `passthrough` route (private tier first) keeps returning the 400, because a rejected request on the
  private target must not silently reach a public provider.
- **Carried like a 413.** `Metered` re-labels a `rejected` answer as a 503 carrying the original status and body, and
  the original is restored when no target answers, so the client still sees the real 400 and message.
- **The bound is distinct providers, counted from `ctx.attempts`.** Before relabelling, `Metered` counts the
  providers whose attempts in this request ended `rejected`; at three (a constant, not configuration), it stops
  with `stop_fallback` instead. Distinct providers rather than attempts so that several models of one strict provider
  count once: they reject for the same reason and say nothing about the request. Three, because two strict providers
  in front of a good one is the case seen (Google AI then Groq), and a request wrong on three is the request.
- **Not a health event, not a speed sample.** A rejection leaves the breaker and the latency ordering alone, so a
  strict member stays ranked where it is and costs one cheap round trip on requests it rejects. `drop_fields` and
  `tool_call_extra_content` remain the way to stop that cost per provider; this change makes their absence survivable
  instead of fatal.
- **Visible.** The attempt outcome is the class name (`rejected`), so decisions, `explain` and the failover metrics
  show it with no new plumbing. The final client error is the last provider's real message.

## Risks

- A malformed request now costs up to three upstream calls on a pool route, not one. They are rejections (no tokens
  generated), the bound is small, and a free pool is where this is acceptable. Paid pools can opt out with an explicit
  `fallback_on`.
- A 400 that means "your prompt violates a content policy" would be tried on up to three providers. Acceptable: the
  client gets the last answer and nothing is generated.
- Hiding a persistent mismatch: a strict member that rejects every Codex request still costs a round trip each time.
  Mitigation: decisions show the `rejected` attempts, and `drop_fields` is documented as the fix.

## Out of scope

Learning field names from error text, a default `drop_fields`, and real thought signatures.
