# 0035. The admin API's time series are read from the decision log at query time

- Status: proposed
- Date: 2026-11-09
- Supersedes: none
- Change: `openspec/changes/add-gui-query-view`

## Context

The admin GUI needs to show how a route, target or endpoint behaves over time, not only in total (a failure rate that began an hour ago reads differently from one that has been steady for a day). The gateway already keeps the record of every request as a decision row with its time and every upstream attempt (ADR 0013), and computes its statistics from those rows when asked. Prometheus metrics exist (ADR 0016) but the GUI is a client of the admin API only (ADR 0025, ADR 0032) and `humpyard ui` has no route to a metrics store; many deployments have none. A chart needs a value per time bucket per group, and the question is where those come from.

## Decision

The admin API's time series are computed at query time from the decision log, by the same counting rules as the totals, through one code path so a series and the totals cannot disagree. Nothing is stored for them: no rollup table, no second write path, no metrics database in the gateway. A series is bounded by the decision retention and by the same row cap as the totals, and when the cap cuts a window short the answer says so and states the time it covers. A time series is offered on the admin API only.

Alternatives that lost: a stored rollup (per-bucket counters written alongside the decision; fast for long windows, but a second write path on the request-completion path, a schema to migrate and a place for the two to drift); reading Prometheus from the GUI (a new dependency and credential, absent in many deployments, and a second source for the same numbers); a client-side series built from `/admin/v1/decisions` (pages of rows moved to the browser to be counted there).

## Consequences

The series needs no migration and adds nothing to the request path; it is exactly as accurate as the decision log and as long as its retention. It costs one more bounded read on the serving process per query, and on a very busy gateway a long window can be cut by the row cap, which the answer reports. If that proves too limiting, stored rollups come back as a new ADR that supersedes this one and keeps the same response shape. Any later view that wants a history (decisions, overrides, cost) starts from the decision log in the same way, and endpoint health stays a current state until something records it.
