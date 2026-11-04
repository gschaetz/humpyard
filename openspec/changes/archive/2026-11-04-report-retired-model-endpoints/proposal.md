# Proposal

## Why

On 2026-10-30 OpenCode Go retired `mimo-v2.5` and began answering it with `410 Model mimo-v2.5 has been deprecated. Use mimo-v2.6-flash instead`. Failover worked and no request failed, but nothing told the operator. The weekly price check and the discovery report compare against the models.dev dataset, which still listed the model, and the discovery report only looks at providers that opt in. The provider's own model list, read on 2026-11-04, had already dropped `mimo-v2.5` (28 models listed, `mimo-v2.6-flash` and `mimo-v2.6-pro` among them), so a comparison against that list would have caught it the day it happened. And when the dataset and the lists miss a retirement, the gateway itself still knows: it classes the answer as `unavailable` (ADR 0023) but exposes no metric for it.

## What Changes

- The gateway counts attempts that failed with the `unavailable` class (404, 410, 401, 402, or a 400 naming a missing model) in a new counter `humpyard_endpoint_unavailable_total{provider,target}`. A retired model, a revoked key and a spent credit become visible on the metrics and alertable from what the provider actually said.
- `humpyard models check` gains `--retired`: it fetches each provider's own `/models` list (the way discovery already does, with the provider's key sent only to its own host) and reports every configured endpoint, hand-written or generated, whose model that list no longer includes. It is report-only, needs no `discover` setting and changes no file. It exits 1 when it reports something, so a scheduled job can alert on it.
- No configuration change, no migration, no new ADR (the counter follows ADR 0023, the check follows ADR 0014 and ADR 0022: third-party data informs the operator and is never fetched while serving).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `metrics`: the endpoint-unavailable counter.
- `cli`: `models check --retired`.

## Impact

- `src/metrics.rs` and the place attempts are recorded (`src/metering.rs`): one counter beside the response-signal counters.
- `src/discover.rs` (or a sibling module) and `src/main.rs`: the provider-list comparison and the `--retired` flag.
- Docs: `docs/metrics.md`, `docs/discovery.md`, the CLI reference, `docs/architecture.md`.
- Cluster side (not this repo): a Prometheus alert on the counter and a weekly job running `models check --retired`, beside the existing price check.
- Not in scope, and dropped from an earlier draft of this change after review: letting discovery add models for a flat-fee plan provider, a per-provider class, and accepting dataset prices on generated targets. Nothing would use such models (the plan routes are hand-picked lists), and discovery adds models while the incident was one going away.
