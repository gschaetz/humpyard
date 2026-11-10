# Proposal

## Why

The first GUI screen (the models view, shipped in 0.29.0) lists every target with every figure the statistics carry: requests, answered calls, failure and fallback rates, four response-signal rates, tokens, cost, three latency percentiles and endpoint health. It is accurate and complete, and the operator who opened it called it "a little overwhelming": nothing says which target needs attention, and a table of totals cannot show whether a failure rate is steady or just started. The operator asked for the shape tools like CloudWatch use: a query bar and a chart on top, details below, with time series because "when did it start" is the first question about any failing target.

## What Changes

- **A time-bucketed statistics endpoint on the admin API.** `GET /admin/v1/stats/series` returns, for the top groups of a window, a series of equal time buckets, each with requests, attempts, failures and failure rate, fallbacks and fallback rate and latency p50, p90 and p99. It takes the same `window`, `group_by` (route, target or endpoint), `route`, `target` and `key` filters as `/admin/v1/stats`, plus `interval` and `limit`. It is computed at query time from the decision log, which already holds every attempt with its time, so there is no new storage and no new write path. Window, bucket count and group count are bounded, and the answer says when the row cap cut the oldest requests off.
- **The models view becomes a query view.** A query bar (window, group by, metric, route and target filters) sits above a chart of the chosen metric and a details table. The chart draws one line per top group over time, with a ranked bar view of the same metric as the alternative; clicking a series filters the table to it.
- **The table shows four headline figures per target**: endpoint health, requests with failure rate, fallback rate and latency. Tokens, cost, answered calls and the response-signal rates move to an expandable row. The default order is problems first (targets that are out or cooling down, then by failure rate, then by fallback rate), so what needs attention is at the top.
- **The query lives in the URL fragment**, so a view can be bookmarked or shared; the fragment is parsed against fixed lists, never trusted.
- **The view refreshes itself.** A refresh control in the query bar offers off, 30 seconds, 1 minute and 5 minutes, and starts at 1 minute. It refreshes only while the page is visible, never starts a request while one is still running, backs off when the gateway fails, stops when the credential is refused, and always shows when the data was last updated. The period is part of the fragment.
- **The chart is plain SVG** drawn by the GUI's own code: the strict Content-Security-Policy (no inline style or script) stays as it is, and no chart library is added.
- The published contract `docs/openapi/admin-v1.json` gains the endpoint, the contract test covers it, and the generated client is regenerated.

Not in this change: the decisions and overrides views, a trend for endpoint health (health is a current state, shown in the table only), downsampled or stored rollups, and any change to `GET /v1/stats` for client keys.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `routing-statistics`: a time-bucketed series over the same groups, windows and filters, with aligned zero-filled buckets, nearest-rank percentiles per bucket and bounds on windows, buckets and groups.
- `admin-api`: `GET /admin/v1/stats/series` on the admin surface with the `read` scope, in the published OpenAPI contract.
- `admin-gui`: the models view is replaced by the query view (query bar, chart, headline table with a detail row, problems-first order, query in the URL fragment); the text-only and strict-CSP rules cover the chart.

## Impact

- `src/ledger/stats.rs` (a bucketing pass over the rows `compute` already reads), `src/server/stats.rs` and `src/server/admin.rs` (the route and its parameters), `docs/openapi/admin-v1.json` and the contract test.
- `ui/`: a query-bar component, an SVG chart component, a reworked table, a URL-fragment state module and a refresh timer; the models view's tests are rewritten around them. No new runtime dependency.
- Docs: `docs/api.md` (the endpoint), `docs/admin-gui.md` (the query view), `docs/architecture.md`.
- Cost: one more read-only query on the serving process, the same size as a `stats` query (the row cap bounds it), now repeated on a timer by every open, visible query view: at most one per 30 seconds per tab. A threat model for it is in `design.md`.
- No change to routing, the ledger schema, client traffic, `humpyard ui`'s forwarding or any existing endpoint's response.
