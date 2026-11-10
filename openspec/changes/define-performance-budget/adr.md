# ADR Review

## Reviewed

- ADR-0005 SQLite ledger with an async batching writer
- ADR-0013 Decisions are recorded per request
- ADR-0016 `/metrics` is served from one metrics registry
- ADR-0025 The UI has no state of its own
- ADR-0027 Admin authority is granted by configuration to named, scoped principals
- ADR-0032 The admin GUI runs as its own process from the same binary
- ADR-0035 The admin API's time series are read from the decision log at query time (proposed, from `add-gui-query-view`; read as a plan and consistent with this change)

No ADR is superseded. This change constrains how ADR 0035's reads are executed and does not reverse it.

## Outcome

One durable decision qualifies: administrative, GUI and statistics work must not change the request path's performance, defined by budgets and checked by a relative isolation test, with heavy reads isolated and the escalation path named. The thresholds, the test's construction, the gate and cache details and the invariant's wording are tactical and stay in `design.md`.

## New ADRs

- [ADR-0036 Admin and UI work must not change the request path's performance](../../../docs/adr/0036-admin-and-ui-work-must-not-change-the-request-paths-performance.md)
