# Design

## Context

Observed on 2026-10-30/31: OpenCode Go answered `mimo-v2.5` with `410 ... deprecated. Use mimo-v2.6-flash instead` (seven failed attempts in 100 hours). `error.rs` already classes 404, 410, 401, 402 and a 400 naming a missing model as `unavailable` (ADR 0023), so failover worked and the cooldown skipped the endpoint. What was missing is a signal. The failure is in the decision log and as a log line; there is no metric, and `HumpyardEndpointCoolingDown` did not fire because a 30 s cooldown rarely holds long enough to alert on.

Two existing checks missed it. `humpyard prices check` and `models check` compare configured endpoints with the models.dev dataset (`configured_drift` in `src/discover.rs`, reported as `gone`; that is how four retired poolside models were found), and `models check` only covers providers that set `discover`. The `opencode` provider sets none, and the dataset still listed `mimo-v2.5`.

Verified on 2026-11-04 against OpenCode Go's `GET /models` with the provider key: 28 models; every model the gateway uses is listed except `mimo-v2.5`, and `mimo-v2.6-flash` and `mimo-v2.6-pro` are listed. The provider's list is therefore a better and faster authority than the dataset for "is this model still offered to this key".

In-force ADRs read: 0004 and 0013 (no content stored), 0009 (health in the pool), 0014 (prices proposed, never fetched at runtime), 0022 (discovery reads the provider's own list and keeps the key to the provider's host), 0023 (answers that describe the endpoint make it unavailable), 0030 (response signals; counters).

## Goals / Non-Goals

**Goals:**
- A retired model, a revoked key or a spent credit shows up as a metric that can be alerted on, from what the provider said.
- A scheduled, report-only check finds configured endpoints the provider has stopped listing, before a request fails.

**Non-Goals:**
- Adding models automatically for any paid or plan provider (ADR 0022 stands unchanged).
- Routing on the counter, or taking an endpoint out of service for longer than the existing cooldown.
- Changing any file: the check only reports.
- The cluster alert and the weekly job (separate repository).

## Decisions

**1. Count `unavailable` attempts, not "410s".** One counter, `humpyard_endpoint_unavailable_total{provider,target}`, incremented once per attempt whose class is `unavailable`, beside the response-signal counters in `observe_usage`/the decision path. The class already holds the cases an operator wants (retired model: 404, 410, a 400 naming a missing model; dead key: 401; no credit: 402), and an alert can be per provider. No `status`, `model` or `key` label: a target name comes from the configuration, and error text must never become a label. Alternative: a fifth stored response signal; rejected because those describe answered calls and are stored in the ledger, while this is about attempts that produced no answer.

**2. `models check --retired` compares against the provider's own list.** For each provider in the configuration that has a usable key (or the one named by `--provider`), fetch `GET <base_url>/models` the way discovery does (the key goes only to that provider's host, bounded size and time, untrusted JSON parsed defensively), and report each configured endpoint whose `model` is not in the list. It uses the same report shapes as `models check` (text and `--json`) and exits 1 when something is reported, 2 when a source could not be used. It needs no `discover` setting and no `price_source`.

**3. Report only.** The check never edits configuration or the generated file and the gateway does not run it. A listing is evidence, not proof: providers can serve models they do not list, so the operator decides, as with every other check.

## Risks / Trade-offs

- **A provider that does not list everything it serves** (aliases, hidden or preview models) would produce false reports. Mitigated by being report-only and by `--provider` filtering; a model-by-model allowance is deferred until a real case appears.
- **A provider without a `/models` endpoint**, or one that rejects the key, is reported as an unusable source (exit 2) for that provider and skipped, as discovery does, so one provider never hides the others.
- **Counter cardinality.** `provider x target` is bounded by the configuration (generated targets are capped at 500).
- **Local providers** (for example a local Ollama) list what is pulled; a configured model that was never pulled would be reported, which is correct.

## Open Questions

None blocking. Whether to also run the check from the gateway's discovery sidecar (it already holds the keys) or a separate weekly job is a deployment choice for the cluster repository.
