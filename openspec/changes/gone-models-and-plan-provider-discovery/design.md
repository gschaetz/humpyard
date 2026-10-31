# Design

## Context

Observed on 2026-10-30/31: OpenCode Go answered `mimo-v2.5` with `410 ... deprecated. Use mimo-v2.6-flash instead` (seven failed attempts in 100 hours); `error.rs` already classes 404, 410, 401, 402 and a 400 naming a missing model as `unavailable` (ADR 0023), so failover worked and the cooldown skipped the endpoint. What was missing is a **signal**: the failure is in the decision log and as a log line, but there is no metric, and `HumpyardEndpointCoolingDown` did not fire (a 30 s cooldown rarely holds long enough to alert on).

Discovery (ADR 0022) is deliberately limited to free chat models, on the reasoning that a wrong member costs a worse pick or a failed attempt, never money, provided no paid model can get in. OpenCode Go is a flat-fee plan that caps each model at a dollar amount of usage priced at list rates (`docs/budgets.md`), so for it "paid" means share of a cap already bought. `models check` does report a hand-written endpoint whose model the dataset no longer lists (`configured_drift` in `src/discover.rs`, shown as `gone`; it is what exits the weekly price check with 1 for four retired poolside models), but only for providers that set `discover`, and only against the dataset. The `opencode` provider sets none, and the dataset still listed `mimo-v2.5`, so nothing was reported.

In-force ADRs read: 0004 and 0013 (no content stored), 0009 (health in the pool), 0014 (prices proposed, never fetched at runtime), 0015 (quota advisory), 0022 (free-pool discovery), 0023 (answers that describe the endpoint make it unavailable), 0030 (proposed; counters for signals).

## Goals / Non-Goals

**Goals:**
- A retired model, a revoked key or an exhausted credit shows up as a metric that can be alerted on, from what the provider actually says.
- A plan provider's new models can join its generated targets without hand-editing, within the same bounds as free pools.
- Providers that are not marked keep today's behavior exactly.

**Non-Goals:**
- Routing on the new counter, or taking an endpoint out of service for longer than the existing cooldown.
- Reporting hand-written targets whose model the provider stopped listing (see Open Questions).
- Adding paid models for pay-per-token providers: still never automatic.
- The cluster alert rule and the cluster `class` change (separate repo).

## Decisions

**1. Count `unavailable` attempts, not "410s".** One counter, `humpyard_endpoint_unavailable_total{provider,target}`, incremented once per attempt whose class is `unavailable`, beside the response-signal counters in `observe_usage`/the decision path. The class already includes the cases an operator wants (retired model: 404/410/400-model-gone; dead key: 401; no credit: 402), and the alert can be per provider. No `status` or `model` label: a model name comes from the configuration only through `target`, and an error body must never become a label. Alternative: a fifth response signal on the ledger; rejected because those are per answered call and stored, while this is about attempts that never produced an answer.

**2. Plan providers are opted in per provider, with their own class.** `discover = { ..., plan = true, class = "plan" }`. The class override is separate from `plan` so a free provider can also get its own class later. Generated plan targets carry the provider's class, so `allowed_classes` on strict routes still removes them.

**3. Plan prices come from the dataset, through the existing strict loader.** Generated targets are priced at zero today and the loader rejects anything else. For plan providers the generator writes the dataset's list price (the same one `prices check` compares) and the loader accepts finite, non-negative numbers within the hand-written bounds, **only** on endpoints of plan providers. This keeps ADR 0014's concern (a wrong price corrupts budgets) bounded: the price only changes cost accounting for a model that was added by an explicit opt-in, and `prices check` continues to report drift against the same source. Alternative: price at zero and let `prices import` fix it by hand; rejected because a zero-priced plan model would hide its use of the cap.

**4. Adding requires both sources, as for free pools.** The provider lists the model **and** the dataset prices it and states a context window. New models carry only the provider's strengths and no declared quality, so they serve `fast` tiers and tag pools but not quality-floored pools (the existing `quality_undeclared` rule). Removal follows the provider's list, as for free models.

**5. The same bounds apply.** `max_added_per_run`, `max_removed_fraction`, `exclude`, the 500-target cap and the duplicate rule are unchanged. A provider's `exclude` is how a plan model that is listed but not enabled on the account is kept out.

## Risks / Trade-offs

- **Listed is not enabled.** A plan provider may list a model the account is not allowed to use (the cluster notes say models were each "confirmed reachable under current account settings"). The generated target would fail with a 4xx on first use, fail over, and, with decision 1, appear on the counter; `exclude` removes it. Accepted: the cost of a wrong member is a failed attempt, as for free pools, and the cap bounds spend.
- **A plan can still run out.** A new model spends the same cap as the others. The quota poll (ADR 0015) stays advisory; nothing here routes on it.
- **Price drift.** A dataset price that lags the provider's real cap accounting only skews the cap-share figure; `prices check` reports it.
- **Counter cardinality.** `provider × target` is bounded by the configuration; generated targets are capped at 500.

## Open Questions

- Does a plan provider's own model list (`/models`) drop a retired model such as `mimo-v2.5`? If it does, `models check` can compare hand-written endpoints against the provider's list as well as the dataset, closing the gap a lagging dataset leaves. Task 1.1 finds out against the live list and records the answer; this change does not depend on it.
- Whether to ship the cluster alert rule with the first release or after a week of counter data is the operator's call in the cluster repository.
