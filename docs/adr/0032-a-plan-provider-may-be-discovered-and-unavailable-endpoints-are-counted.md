# 0032. A flat-fee plan provider may be discovered, and unavailable endpoints are counted

- Status: proposed
- Date: 2026-10-31
- Supersedes: none (refines [0022](0022-free-pool-membership-may-be-updated-on-a-schedule-without-review.md): its rule that nothing is added for a paid provider)
- Change: `openspec/changes/gone-models-and-plan-provider-discovery`

## Context

ADR 0022 lets free pools update themselves because a wrong free member costs a worse pick or a failed attempt, never money, and it therefore never adds a model for a paid provider. A flat-fee plan such as OpenCode Go is paid in a different way: the money is spent whatever is used, each model is capped at an amount of usage, and a new model draws on that existing cap. On 2026-10-30 that plan retired a model the pool still listed; failover hid it, but nothing reported it, because the scheduled checks read a dataset that lagged and discovery does not look at hand-written targets.

## Decision

A provider may be marked as a **plan** provider (`discover.plan`). For it only, discovery may add a chat model that the provider lists and the dataset prices, as a generated target priced at the dataset's list rate and carrying the provider's own class, under the bounds ADR 0022 already sets (opt-in at two levels, per-run add limit, removal fraction, exclude list, strict loader). The strict loader accepts a non-zero price only on endpoints of plan providers. Providers that are not marked behave exactly as before, and a pay-per-token provider is still never added to automatically. Independently, the gateway counts every attempt that fails with the `unavailable` class in a metric labeled only by provider and target, so a retired model, a revoked key or a spent credit can be alerted on from what the provider says.

Alternatives considered: treating every paid provider alike (a pay-per-token model can cost without bound, which a plan cap does not); pricing generated plan targets at zero (hides use of the cap); adding a fifth stored response signal for gone models (those describe answered calls); probing each new model with a real call before adding it (spends the cap and sends a prompt on a schedule).

## Consequences

A plan's lineup can follow the provider without editing, and the operator's levers (the exclude list, the add limit, the class) stay. A listed-but-not-enabled model can join and fail until excluded; the counter makes that visible within a request. The price on a generated endpoint now comes from third-party data, bounded by the loader's checks and `prices check`. Hand-written plan targets remain the operator's to watch, with the new counter as the signal.
