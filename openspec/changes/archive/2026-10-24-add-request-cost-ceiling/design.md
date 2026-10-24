# Design

## The ceiling and the rule for applying it

`max_request_usd` is a positive number of US dollars on a key. A target is removed from a request's candidates only
when the request is certain to cost more than the ceiling on it. The estimate is a lower bound, built from parts
that can only under-count:

- **Prompt tokens:** the existing low estimate (`estimate_tokens_low`), which stays below any real tokenizer's
  count.
- **Output tokens:** the output length the client requested (`max_tokens` or `max_completion_tokens`), or zero
  when it asked for none. A client that sets no limit is not charged an assumed output, so the rule never
  refuses what it cannot prove too dear.
- **Rates:** the lowest input and output rate among the target's endpoints and tiers that could serve the request,
  so a target is removed only if even its cheapest endpoint is too dear.

This is the same principle as ADR 0019: err toward serving, remove only on proof. The cost is that the ceiling is a
guard against clearly oversized or runaway requests, not a precise cap; a request with no stated output limit can
still cost more than the ceiling after it runs, and the existing budgets and their bounded overshoot cover that.

## Where it applies

It is one more input to the planning stage, a policy that only subtracts, evaluated after the key's budget state
and before health, alongside the other eligibility checks. It does not reorder anything and never adds a target.
Free targets (every endpoint priced zero) have a lower bound of zero and always pass. A target with a price
`tiers` list uses the lowest applicable rates for the lower bound.

## Refusal

When the ceiling removes every candidate, the request is refused before any upstream call with HTTP 402, error code
`request_cost_ceiling`, and a message giving the ceiling and the lower-bound estimate for the cheapest candidate.
It is a different code from `budget_exhausted` so a client can tell "this request is too big" from "your
allowance is used up".

## Validation

Like a USD budget, a key with `max_request_usd` requires every endpoint to carry a price (zero for free models), so
nothing is silently free. A non-positive value is rejected at load.

## Observability

The decision records the ceiling and the targets it removed; `explain` lists them with the lower bound; the metric
`humpyard_cost_ceiling_total{outcome="removed"|"refused"}` counts requests where the ceiling removed targets or
refused. Key labels follow the existing `key_labels` setting.

## Profiles

When key profiles exist (ADR 0025), `max_request_usd` is a profile ceiling that a database-managed key may lower
and never raise; this change defines the setting only on a configuration key.
