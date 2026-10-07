# Design

## Context

Phase 1 proxies to one upstream. Switchyard 0.3.0 findings (docs/background.md): libsy makes no
network calls; `Algorithm::run_stream` yields offloaded model calls plus a final
`RoutingOutcome { selected_model_ids, request, ... }`. `switchyard-llm-client::run` drives that
stream against a `ClientRouter` mapping each target `ModelId` to a `RoutedLlmClient`
(`async fn call(Request) -> Result<Response, LlmClientError>`), with per-target request
preparation and ordered fallback. Session state lives inside long-lived algorithm instances
(keyed by `Metadata.session_id`), not in the host. `AlgorithmSpec` (runner crate) is
deserializable, but its builder is crate-private.

## Spike findings (verified by `tests/switchyard_assumptions.rs`)

- **Per-request model filtering works.** Algorithms read targets from `RuntimeModels` grouped by
  `Category` (`Any`, `Capable`, `Efficient`, `Judge`, named). Removing a target from the map hides
  it; an empty list makes the run fail (`no routing targets are configured`).
- **`Category::Any` must contain every target**: algorithms validate their picks against it
  (`TargetNotFound` otherwise). The filter must therefore remove an ineligible target from
  *every* category, and rebuild `Any` as the union.
- **A tier emptied by the filter does not degrade on its own**: a stage router whose `capable`
  list is empty fails with `no models available for category capable`. The policy layer must
  substitute (e.g. capable falls back to the eligible efficient targets) so budget limits
  degrade routing instead of erroring. Only when nothing is eligible do we return 503.
- **`run` streams the final answer.** It returns as soon as the answer call yields a response;
  the `LlmResponse::Stream` is live and unconsumed. Only routing-time (judge) calls are buffered.
  `Response.metadata.served_model` names the serving target even for streams.
- **Session state persists** inside one shared algorithm instance, keyed by
  `Metadata.session_id`: a stage-router escalation holds capable for the next request of the same
  session and does not affect other sessions.
- **Failover exists upstream.** `run` tries `selected_model_ids` in order and advances on
  transport errors, `TemporarilyUnavailable`, context-window errors, HTTP 403/408/429/5xx and
  content-policy 400s; other 4xx stop. Retries and cooldowns live in `TranslatingLlmClient`.
- **`TranslatingLlmClient` is a ready-made `RoutedLlmClient`**: per-model HTTP backends for
  OpenAI Chat, Responses and Anthropic, with retries, timeouts, streaming and translation.

## Goals / Non-Goals

**Goals:** built-in algorithms driving real routing; provider failover; a policy seam that later
budget/health work plugs into without touching algorithms.

**Non-Goals:** custom algorithms, active health probing, cost accounting, hot reload of config.

## Decisions

- **Implement `RoutedLlmClient` per target on our provider pool** and call `switchyard-llm-client`'s
  `run`/`ClientRouter`, instead of writing our own `drive` loop. Keeps classifier calls, request
  preparation and target fallback upstream-maintained. Alternative: own driver; rejected until a
  need appears.
- **Dispatch moves onto Switchyard's client.** Each endpoint becomes a `TranslatingLlmClient`
  (OpenAI Chat backend, key from env) replacing the phase-1 hand-written reqwest relay and its
  SSE parsing. Decode/encode of the *client* side stays on `switchyard-translation`.
- **A target is one `ModelId` served by a small wrapper client** that walks the target's ordered
  endpoints (setting each endpoint's upstream model name) and fails over using the same
  conditions as `run` (transport, unavailable, 403/408/429/5xx, context window). Keeping a target
  a single `ModelId` preserves target-level semantics for `random` weights and stage-router tiers;
  one-id-per-endpoint would skew them. The wrapper adds the serving provider to
  `Response.upstream_headers` so it can be reported. No failover once a stream has started.
- **Build algorithms ourselves with libsy constructors** (`Passthrough`, `Random`, `StageRouter`,
  `LlmTaskClassifier`) from our own TOML route definitions, reusing `AlgorithmSpec`'s field
  names where practical so Switchyard docs apply. Alternative: depend on `switchyard-runner`'s
  `Runner`; rejected because it builds its own HTTP clients and would bypass our pool.
  Algorithm instances are built once at startup and shared via `Arc` so session state persists.
- **Policy seam = filter on `RuntimeModels`** (confirmed by the spike): per request, build the
  `Arc<RuntimeModels>` from the eligible targets only (trait `eligible(&PolicyContext) ->
  EligibleTargets`), removing ineligible targets from every category, rebuilding `Any`, and
  substituting for emptied tiers (capable → eligible efficient). Default impl returns everything.
- **Direct target requests** become an implicit `passthrough` route built at startup.
- **Attribution** set from the `(ModelId, provider)` the winning client reports, via an internal
  response extension, rendered as headers for both streaming and buffered responses.
- **Pipeline stays decode → route → dispatch → encode** from phase 1; routing inserts between
  decode and dispatch, so the three client protocols are unaffected.

## Risks / Trade-offs

- Tier substitution changes what an operator configured (capable silently served by efficient) →
  expose it in logs and in `x-conductor-target`; make it the documented budget-degradation behavior.
- Our failover conditions mirror upstream's private `fallback_reason` → covered by tests; revisit on upgrades.
- Switchyard 0.x API churn → exact version pins and one `routing` module as the only importer.
- Known upstream issue: buffered upstream work continues after client disconnect → ensure our
  client drops the request future on cancellation; test it.

## Migration Plan

Breaking config change; update `examples/config.toml` and README. No data to migrate.

## Open Questions

- Whether to also expose Responses `previous_response_id` once dispatch runs on `run`, which
  already tracks Responses state per target. Deferred; phase-1 behavior (400) stays.
