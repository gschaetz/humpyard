# Design

## Context

The statistics (`routing-statistics`) are computed on read from the decision log and the usage rows (ADR 0013). One
aggregate query already sums, per group, the answered calls, their cost, their tokens by kind (for the calls whose usage
the provider reported) and the four response signals. Statistics are a heavy read: one at a time per surface, behind the
shared ten-second result, folded off the serving runtime (ADR 0036). A price is `input`, `output`, an optional cached
rate and optional tiers by prompt size (`pricing.rs`); the session report already prices abandoned tokens at a
destination endpoint's prices and reports how many calls were priced and unpriced. See proposal.md for the motivation.

## Goals / Non-goals

**Goals:**
- A number the operator can defend: what these calls would have cost at one named endpoint, from the ledger's own tokens.
- No new work on the request path and no new row read.

**Non-goals:**
- A default baseline in the configuration, tiered baseline prices, the GUI, the `ctl` command.
- Any claim about answer quality, or that the baseline endpoint could have served the traffic.

## Decisions

**1. The baseline is a query parameter, `provider/model`.** It uses the name that `group_by=endpoint` already shows, so a
figure can be reproduced by asking for the same name. It is looked up among the configured targets' endpoints, the first
target that names it winning, as the session report does. The operator picks it per question, so the answer always says
what it was compared with. *Alternative:* a configured default (one more setting, restart or reload rules, and a figure
that changes meaning when someone edits the file); kept for a later change.

**2. Admin only.** A baseline answer shows an endpoint's prices. The admin API already shows the whole configuration; a
client key must not learn the prices of endpoints it may not use, so `GET /v1/stats` refuses the parameter.

**3. Untiered prices only.** A call's cost under a tiered price depends on that call's prompt size, so it cannot be had
from per-group sums. With no tiers the cost is linear in the token counts, so the per-group sums give the exact figure
with no extra read. A tiered baseline is refused with a message saying why. *Alternative:* read the usage rows and price
each (a second heavy read, and a cap that would make the figure partial); not worth it for a baseline an operator can
choose to be untiered.

**4. The same calls on both sides.** The baseline prices the answered calls whose usage the provider reported, and the
actual cost is the cost of those same calls (a call with no reported usage has a cost of zero in the ledger, so it is
excluded from both and counted). Classifier and judge calls are not answer calls and are in neither, as in every other
cost figure of the statistics. Without this a saving could be produced by a missing record.

**5. The cost rules are the ledger's own.** Fresh input and cache-creation tokens at the input price, cache reads at the
cached price or the input price, output tokens (reasoning included) at the output price, as `cost_micro_usd` does. The
baseline function is that arithmetic over sums, kept next to it so the two cannot drift apart, and a test prices one call
both ways.

**6. A summary and a statement, not a headline.** The answer adds up the groups into a `summary` (the figure someone will
quote) and carries a `basis` sentence and the count of calls left out, so the number travels with what it is. It reports
the saving and its ratio and never a "you saved" string. Nothing is named "savings" in the field names beyond `saved_usd`
(the difference), so a reader sees the arithmetic.

## Threat model

Assets: the configured prices and endpoint names, which a client key must not learn beyond what it may use. Actors: a
client key (refused), a credential with `read` scope (already sees the configuration), and anyone who can reach the admin
listener (already needs a credential).

| Threat | Mitigation | Proof |
|---|---|---|
| A client key learns the price of an endpoint it may not use, by guessing a baseline | The client statistics refuse the parameter | A test that `GET /v1/stats?baseline=` is a 400 and reads nothing |
| A hostile baseline string reaches a query or a response | It is matched exactly against the configured endpoint names and never reaches SQL; it is echoed only inside a JSON string | A test with a hostile value |
| A baseline answer is cached for a request with another baseline | The shared result is keyed by the whole parameter set | A test that two baselines do not share a result |
| A figure is quoted without its basis | The answer carries the baseline, its prices, the statement and the calls left out | A test that the fields are always present with a baseline |

## Risks / Trade-offs

- [A figure read as a promise] → The wording is "would have cost", the answer states the basis, and the documentation
  says what it leaves out (answer quality, a baseline's own tokenizer and caching).
- [A baseline's tokens differ from the serving model's] → Stated in the basis; the estimate uses the tokens the calls
  actually used, which is the only count the ledger has.
- [Heavy read cost] → Two sums added to the existing aggregate and arithmetic over the groups; one isolation run records
  that nothing moved.

## Migration Plan

None. The parameter is optional and the answer without it is unchanged; nothing is stored.

## Open Questions

None that change the specs or the tasks. A configured default baseline and a GUI column are later changes.
