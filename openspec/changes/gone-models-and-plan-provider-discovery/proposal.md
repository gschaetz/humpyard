# Proposal

## Why

On 2026-10-30 OpenCode Go retired `mimo-v2.5` and began answering it with `410 Model mimo-v2.5 has been deprecated. Use mimo-v2.6-flash instead`. Failover worked and no request failed, but nothing told the operator: the weekly price check reads the models.dev dataset (which still lists the model) and discovery only maintains free pools, so a hand-written plan target that goes away is invisible until someone reads logs. Separately, OpenCode Go is a flat-fee plan capped per model, not per-token billing, yet discovery treats every paid provider as "adding a model costs money" (ADR 0022) and never adds anything. On a plan, a new model draws on a cap that already exists, so that rule is stricter than the risk it guards against.

## What Changes

- The gateway counts attempts that failed because the endpoint cannot serve at all (the existing `unavailable` failure class: 404, 410, 401, 402, or a 400 naming a missing model) in a new counter `humpyard_endpoint_unavailable_total{provider,target}`. A retired model, a revoked key and an exhausted credit become visible on the metrics and alertable, without waiting for the next scheduled check and without relying on a dataset.
- A provider can be marked as a **plan** provider. For it, `models sync` MAY add a chat model that the provider lists **and** the dataset prices, as a generated target priced at the dataset's list rate (cost means share of the cap, as `docs/budgets.md` already says) and carrying the provider's class (for example `plan`), within the same bounds as for free providers (`max_added_per_run`, `exclude`, strengths, strict loader). Nothing changes for providers that are not marked.
- The generated-file loader accepts a non-zero price **only** on endpoints of a plan provider, with finite, sane numbers; everything else stays priced at zero.
- Out of scope, kept as a documented follow-up: reporting a *hand-written* target whose model the provider stopped listing (depends on whether the provider's own model list drops retired models; the design records how to find out), routing on the new counter, and changing the `class` of existing targets (done in the cluster configuration, not here).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `metrics`: the endpoint-unavailable counter.
- `gateway-config`: the plan marking on a provider's `discover`, per-provider class, and the loader accepting dataset prices on plan providers' generated endpoints only.
- `cli`: `models sync` and `models check` for plan providers.

## Impact

- `src/metrics.rs` and the place attempts are recorded (`src/metering.rs`): one counter beside the response-signal counters.
- `src/discover.rs`, `src/config/schema.rs`, `src/config/validate.rs` and the generated-file loader: the `plan` flag, the per-provider class, the price rule.
- Docs: `docs/metrics.md`, `docs/discovery.md`, `docs/configuration.md`, `docs/architecture.md`; a new ADR 0032 that refines ADR 0022.
- Cluster side (not this repo): a Prometheus alert on the counter and opting OpenCode Go in; both are separate changes there.
- No migration: both additions are opt-in and absent by default.
