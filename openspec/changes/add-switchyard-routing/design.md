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

## Goals / Non-Goals

**Goals:** built-in algorithms driving real routing; provider failover; a policy seam that later
budget/health work plugs into without touching algorithms.

**Non-Goals:** custom algorithms, active health probing, cost accounting, hot reload of config.

## Decisions

- **Implement `RoutedLlmClient` per target on our provider pool** and call `switchyard-llm-client`'s
  `run`/`ClientRouter`, instead of writing our own `drive` loop. Keeps classifier calls, request
  preparation and target fallback upstream-maintained. Alternative: own driver; rejected until a
  need appears.
- **Failover lives inside the target client**: a target's `RoutedLlmClient` walks its endpoint
  list and fails over (429, 5xx, connect, timeout) before returning an error, so Switchyard's
  fallback list only handles whole-target failure. Spec: no failover after first streamed byte.
- **Build algorithms ourselves with libsy constructors** (`Passthrough`, `Random`, `StageRouter`,
  `LlmTaskClassifier`) from our own TOML route definitions, reusing `AlgorithmSpec`'s field
  names where practical so Switchyard docs apply. Alternative: depend on `switchyard-runner`'s
  `Runner`; rejected because it builds its own HTTP clients and would bypass our pool.
  Algorithm instances are built once at startup and shared via `Arc` so session state persists.
- **Policy seam = filter on `RuntimeModels`**: per request, build the `Arc<RuntimeModels>` passed to
  `run_stream` from the eligible targets only (policy is a trait `eligible(&PolicyContext) ->
  EligibleTargets`). Default impl returns everything. Alternative: wrap `Algorithm` and rewrite
  `selected_model_ids`; kept as fallback if `RuntimeModels` filtering cannot express a route.
- **Direct target requests** become an implicit `passthrough` route built at startup.
- **Attribution** set from the `(ModelId, provider)` the winning client reports, via an internal
  response extension, rendered as headers for both streaming and buffered responses.
- **Pipeline stays decode → route → dispatch → encode** from phase 1; routing inserts between
  decode and dispatch, so the three client protocols are unaffected.

## Risks / Trade-offs

- `RuntimeModels` filtering may not hide targets that an algorithm was configured with explicitly
  (e.g. a stage router's fixed tiers) → Spike first (task 1.1); fall back to the wrapper approach.
- `run` buffers routing-time calls and decides before the answer call; streaming answers may need
  `decide` + our own dispatch instead of `run` → Spike confirms which entry point streams.
- Switchyard 0.x API churn → exact version pins and one `routing` module as the only importer.
- Known upstream issue: buffered upstream work continues after client disconnect → ensure our
  client drops the request future on cancellation; test it.

## Migration Plan

Breaking config change; update `examples/config.toml` and README. No data to migrate.

## Open Questions

- Which entry point (`run` vs `decide` plus own dispatch) gives true streaming of the final answer;
  resolved by the spike before implementation continues.
