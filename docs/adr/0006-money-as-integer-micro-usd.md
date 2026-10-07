# 0006. Money is integer micro-USD

- Status: accepted
- Date: 2026-10-07 (recorded retroactively; decided 2026-10-07)
- Supersedes: none
- Change: [add-cost-tracking](../../openspec/changes/archive/2026-10-07-add-cost-tracking)

## Context

Costs are sums of many tiny products (tokens times a per-million-token price). Floating-point sums
drift, and the ledger is summed to enforce budgets.

## Decision

Prices are declared in USD per million tokens, which is numerically micro-USD per token, so a
call's cost is `tokens x price` rounded to the nearest whole micro-USD and kept as an integer. The
ledger stores `cost_micro_usd INTEGER`, the budget counters hold integers, and limits are compared
in micro-USD. Floating point appears only where humans meet the system: parsing prices and limits
from config and rendering dollars in `/v1/key/info`.

Alternatives considered: `f64` dollars (drift); a decimal crate (a dependency for no gain at this
scale); cents (too coarse for sub-cent calls).

## Consequences

- Sums of stored costs are exact. Each call is rounded to the nearest micro-USD, so the error is
  at most half a micro-USD per call; it can add up over very many calls but is negligible next to
  budgets measured in dollars.
- Changing the unit later would be a data migration of the ledger.
- Casts between integer and float types in money and token paths need care (hardening group 3
  adds lints and tested helpers; invariant 9 in [invariants.md](../invariants.md)).
