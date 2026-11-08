# Design

## Context

ADR 0019 lets an endpoint declare `context_window`, `tools` and `vision`; in planning a target is removed when none of its endpoints can serve the request, and only on certainty (`incapable_reason` in `src/server.rs`; the prompt comparison uses the deliberately low estimate, `Features::prompt_tokens_low`). The low estimate is computed only when something needs it (`declares_context_windows`, or a key with a cost ceiling). A pool's `min_context_window` filters on `context_window` at load (`config/pools.rs`). The removal reason is a bounded label (`CAPABILITY_REASONS` in `src/metrics.rs`, ADR 0016) and is stored as text on the decision, so a new reason needs no ledger change. Inside a target, endpoints are still tried in order; skipping single endpoints is not done (ADR 0019). See proposal.md for the motivation.

## Goals / Non-Goals

**Goals:**
- Keep "what the model can read" and "what this provider will take in one request" as separate declarations, so each does its own job.
- Reuse the capability machinery unchanged in spirit: certainty only, removal only, recorded, counted, refused with a reason.

**Non-Goals:**
- Counting the requested output toward the cap. Whether a provider counts `max_tokens` against its limit is unverified; the comparison uses the prompt alone and the gap is recorded below.
- A per-minute token bucket. A prompt under the cap can still be refused when the minute's allowance is spent; that is a 429 and the stated-retry-time cooldown already handles it.
- Skipping an endpoint inside a target, learning a cap from a 413, and importing caps from public data.

## Decisions

**1. A separate endpoint field, `max_request_tokens`.** Alternative: keep `context_window` and drop Groq from large-window pools, as the stopgap does. Rejected: it loses the endpoint for prompts it can serve. Alternative: learn the cap from the 413 body. Rejected: it parses provider-specific text and only learns after a failed request.

**2. Same rule as the window: certainty, per target.** A target is removed only when every one of its endpoints cannot serve the request, and an endpoint whose cap the low estimate exceeds is one that cannot; an endpoint with no cap, or a cap the prompt fits, keeps the target. This keeps ADR 0019's "any endpoint is enough" and means a multi-endpoint target with one capped endpoint still tries it first and relies on the existing failover. That is the same limit the window check has today; fixing it for both is a later change.

**3. A distinct reason, `request_cap`.** Reusing `context` would make "the model's window" and "the provider's cap" indistinguishable in decisions and the counter, which is the confusion this change removes. `CAPABILITY_REASONS` grows by one bounded value. The refusal sentence names `max_request_tokens`.

**4. Validation.** `max_request_tokens` must be at least 1, and no greater than the endpoint's `context_window` when both are declared: a cap above the window means nothing and is most likely two swapped numbers. A bad value (zero, negative, fractional, not a number) is refused while the file is parsed, naming the setting; a cap above the window is refused in validation, naming the target, endpoint and setting.

**5. The low estimate is wanted when a cap is declared.** `declares_context_windows` becomes true for an endpoint that declares either, so a configuration with only caps measures the prompt.

**6. The pool filter is unchanged.** `min_context_window` keeps reading `context_window`; that is the point of the separate field.

## Risks / Trade-offs

- **A wrong cap refuses requests** (declared too low) → the message names `max_request_tokens` and the target, and the field is operator-declared, never imported.
- **Prompt-only comparison under-removes** if a provider counts the requested output too → a request near the cap can still be refused upstream; the existing overflow fallback handles it, as it does for the window. Verifying Groq's behavior is deferred.
- **A prompt under the cap still hits a per-minute allowance** → out of scope; a 429 with a stated retry time already cools the endpoint.
- **Stale declaration** → a provider that raises its cap needs the operator to edit it; the same property `context_window` has.

## Migration Plan

Nothing changes until `max_request_tokens` is set. To replace the stopgap, set `context_window` back to the model's real window and add `max_request_tokens = 8000` on the three Groq endpoints, then reload. Rolling back is removing the field.
