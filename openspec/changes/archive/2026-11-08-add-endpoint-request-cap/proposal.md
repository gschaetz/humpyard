# Proposal

## Why

An endpoint's `context_window` says what its model can read. Some providers also cap what one request may
carry, far below that: Groq's free models declare a window of 131072 but accept about 8,000 tokens per request,
and answer 413 above it. The gateway has one number for both, so the operator must pick: declare the window and
pay a wasted round trip on every oversized request (which then fails over), or declare the cap as the window and
lose the endpoint from every pool that asks for a large window (`min_context_window`), even for small prompts it
could serve. Keeping the two numbers apart fixes both.

## What Changes

- A new optional `max_request_tokens` on an endpoint: the most prompt tokens one request may carry to that
  provider. Undeclared means no cap.
- Planning treats it like `context_window`: a target is left out of a request when a deliberately low estimate of
  the prompt exceeds the cap of every one of its endpoints (the rule of ADR 0019, certainty only). The request is
  never sent to a target that is certain to refuse it.
- A pool's `min_context_window` filter keeps reading `context_window` only, so an endpoint with a large window
  and a small cap stays a member of the large-window pools.
- The removal has its own reason, `request_cap`, in decisions, `explain`, the refusal message and
  `humpyard_capability_skips_total`. If nothing can take the request, it is the existing 400
  `no_capable_target`.
- Existing configurations are unaffected: no cap, no change. After upgrading, the operator can restore
  `context_window = 131072` on the Groq endpoints and add `max_request_tokens = 8000`.
- **Not in this change:** a cap that counts the requested output (`max_tokens`); whether Groq counts it toward its
  limit is unverified. Skipping individual endpoints inside a target (ADR 0019 leaves that for later). Learning a
  cap from a provider's error. A cap proposed by `prices import`.

## Capabilities

### New Capabilities
None.

### Modified Capabilities
- `routing-policy`: an endpoint may declare a per-request cap and a target is removed for it, only when certain.
- `gateway-config`: the `max_request_tokens` setting and its validation.
- `metrics`: the capability-skip counter gains the reason `request_cap`.

## Impact

`src/config` (the endpoint field and its validation), `src/server.rs` (the low estimate is wanted when a cap is
declared; the incapable check and its sentence), `src/metrics.rs` (one more bounded reason), `src/server/explain.rs`
if it names reasons, `docs/routing.md`, `docs/configuration.md`, `docs/metrics.md`, `docs/architecture.md`. No ledger
schema change: a decision stores the reason as text already.
