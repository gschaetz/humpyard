# 0036. Admin and UI work must not change the request path's performance

- Status: proposed
- Date: 2026-11-09
- Supersedes: none
- Change: `openspec/changes/define-performance-budget`

## Context

humpyard's product is the request path: a gateway that adds almost nothing to a call and never loses a record of it. The admin API, the GUI and the statistics views read the same ledger, run on the same runtime and share the process's CPU allowance. Their cost was recorded (`docs/performance.md`) but never bounded: the statistics read folds up to 50,000 decision rows' JSON on a serving worker without yielding, through a two-connection ledger pool shared with the usage writer, whose queue drops entries when full. The cluster gives the gateway container 500m of CPU, which may leave the runtime one worker. Each new admin or UI feature adds another reader of these resources, and without a stated target and a test a regression would be found by a user. The author's own traffic is tiny, so there is no pain to push back on: the rule has to be written down while it is cheap.

## Decision

Administrative, GUI and statistics work must not change the request path's performance, and the project states what that means and checks it.

- Performance is defined by budgets in `docs/performance.md`, each with a value, a basis and an enforcer. The first are gateway-added latency (p50 at most 2 ms and p99 at most 10 ms against a 50 ms upstream at up to 64 concurrent requests) and throughput (at most 10% below the previous release's, measured alongside it in one session, because a recorded figure from another day is as noisy as the tolerance); measures that are unmeasured are listed as such and get budgets after a measurement run.
- A relative isolation test, run in CI, fails the build when looping heavy reads move request p99 by more than 10% or 1 ms (the larger) on a one-worker runtime, or when any ledger entry is dropped. Absolute budgets are checked at release on a reference machine, because shared runners are too noisy for them.
- Heavy ledger reads run off the serving runtime, on a read-only connection pool of their own, one at a time per surface (admin and client), shed with 503 and `Retry-After` instead of queueing, with identical reads sharing one result for ten seconds. The rule is a numbered invariant with an architecture test, so the wrong code fails the build.
- A change that adds work to the request path or uses a shared resource (the serving runtime, the ledger pool, the writer's queue, the CPU allowance) carries a performance task and a measurement.
- The named escalation, if the isolation test still fails with those layers: heavy reads move to a separate process working from a local snapshot of the ledger (SQLite backup API or `VACUUM INTO`). It is never a second process opening the live WAL ledger, which the cluster keeps on NFS where WAL's shared memory is unsupported. That step would be its own ADR.

Alternatives that lost: stored rollups (a second write path and a schema, and ADR 0035 chose against them); a live read replica (WAL on NFS is unsupported); absolute thresholds in CI (a margin wide enough to be stable is too wide to catch what matters); a single shared gate (lets a client key shut out the operator); queueing heavy reads (the backlog is the problem).

## Consequences

Statistics can be up to ten seconds old and can be answered 503 when another heavy read is running; callers retry after `Retry-After`. Every future read-heavy feature (the decisions and overrides views, longer windows) has a test it must pass and a place in the gates, and cannot quietly spend the request path's budget. The cost is a CI job, a handful of tests that read source text, and a documented procedure at release. What we now must keep true: the thresholds stay in the spec and `docs/performance.md`, the isolation job stays required, and a measure is not claimed anywhere without a recorded run.
