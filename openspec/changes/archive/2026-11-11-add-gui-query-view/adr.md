# ADR Review

## Reviewed

- ADR-0013 Decisions are recorded per request
- ADR-0016 `/metrics` is served from one metrics registry
- ADR-0025 The UI has no state of its own
- ADR-0027 Admin authority is granted by configuration to named, scoped principals
- ADR-0030 Response quality is recorded as content-free flags per call
- ADR-0032 The admin GUI runs as its own process from the same binary

No ADR is superseded; all of the above are in force.

## Outcome

One durable decision qualifies: where the admin API's time series come from (computed from the decision log at query time, no stored rollups, admin-only). The query bar, the chart, the table order and the URL fragment are tactical and stay in `design.md`.

## New ADRs

- [ADR-0035 The admin API's time series are read from the decision log at query time](../../../docs/adr/0035-the-admin-apis-time-series-are-read-from-the-decision-log-at-query-time.md)
