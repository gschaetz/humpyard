# Proposal

## Why

Two things in the query view read wrongly in daily use. Every time is shown in UTC, so the operator converts in their
head to find when something started, and the chart's one question is "when did it start". And the ranking of bars is
drawn in the table's problems-first order, so under the requests or latency metric the longest bar is not on top, and
under a rate metric a short bar can sit above a long one. A ranking should be ordered by the figure it shows.

## What Changes

- The query bar gains a time choice, `Local` (the browser's time zone) or `UTC`, starting at `Local`. It changes the
  chart's axis ticks, the hover text of a point and the axis note, which names the zone in use. It is kept in the URL
  fragment with the rest of the query and read against a fixed list like the other choices. Data, buckets and the API
  are unchanged; the gateway still aligns buckets to the epoch.
- The ranked bar view is ordered by the chosen metric's value, highest first, with groups that have no value last and ties
  by name. A bar that stands on fewer than 20 calls says how many calls, so a rate from one call is not mistaken for a
  trend. The table keeps its problems-first default.

## Capabilities

### New Capabilities

None.

### Modified Capabilities
- `admin-gui`: a time-zone choice, a ranking ordered by its metric, and the fragment's fixed lists gain the zone.

## Impact

- **Code:** `ui/src` only (the query model, the chart's time formatting, the query bar, the bar view), their tests and
  `docs/admin-gui.md`.
- **API, server, ledger, request path:** none. The performance register selects only the UI checks.
- **Security:** the new fragment value is read against a fixed list (`local` or `utc`); any other value is the default.
