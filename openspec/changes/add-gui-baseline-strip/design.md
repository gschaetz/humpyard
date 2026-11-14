# Design

## Context

The query view (`add-gui-query-view`, refined by `refine-gui-query-view`) reads statistics, the series and health on a
refresh timer, keeps its query in the URL fragment against fixed lists, and renders every API string as text under a strict
policy. `add-baseline-cost` added `baseline=<provider>/<model>` to `GET /admin/v1/stats` and a `summary` and `baseline`
object to its answer. The effective configuration (`GET /admin/v1/config`, `read` scope) lists each target's endpoints with
their prices. See proposal.md for the motivation.

## Goals / Non-goals

**Goals:**
- Show the figure beside the traffic it is about, with what it is and is not.
- No server change and nothing written.

**Non-goals:**
- A default baseline in the configuration, several baselines at once, a table column.
- Anything beyond the query view.

## Decisions

**1. The choices come from the gateway's own configuration.** The list is every endpoint of every target that has a price
with no tiers, as `provider/model`, first seen winning, most expensive output price first (a baseline is usually the model
you would otherwise have used for everything). The fragment value is read against that list, which is the same rule as the
other choices: a value that is not offered is none. The configuration is read once when the view starts and again on a
manual refresh, because it changes only with a reload; a failed read hides the choice and nothing else.

**2. The statistics carry the baseline, the series does not.** The chart is over time and does not need it; the figure is
a whole-window summary, so it is read with the statistics the table already reads. A change of baseline reads again like
any other choice of the query.

**3. Words, not a dashboard tile.** The strip is a sentence or two: "At openai/gpt-5 ($1.25 in, $10 out per million) this
traffic would have cost $1.84. It cost $0.07: $1.77 less (96%)." with the estimate statement and the left-out count in
smaller text below. A figure alone invites quoting without its basis, which is what the API's `basis` text exists to
prevent. When the baseline would have been cheaper it says "more".

**4. An older gateway is a notice, not an error.** A gateway that predates `baseline` answers 400 for the unknown
parameter. The page then shows a notice, drops the baseline from the query it uses and carries on, as it does for a
gateway with no series endpoint.

## Threat model

| Threat | Mitigation | Proof |
|---|---|---|
| A crafted link sets a baseline to markup or another value | The fragment value is accepted only when it is exactly an offered endpoint, otherwise none | A test with a hostile fragment |
| The endpoint name or statement from the API injects markup | Rendered as text nodes only, no raw HTML (the lint rule bans it) | A test with markup in both |
| A link makes the page request something else | The value only ever becomes a query parameter of the existing statistics call, encoded | A client test that it is encoded |

## Risks / Trade-offs

- [The figure is read as a promise] → The strip states what it is next to the number, and "more" is said plainly when it
  applies.
- [A long list of endpoints in the choice] → Sorted by price and shown as names with prices; a gateway with dozens of
  priced endpoints is rare enough to accept a long select for now.

## Migration Plan

None. The new fragment value is optional and its absence means none.

## Open Questions

None.
