# 0005. SQLite usage ledger with an asynchronous writer

- Status: accepted
- Date: 2026-10-07 (recorded retroactively; decided 2026-10-07)
- Supersedes: none
- Change: [add-cost-tracking](../../openspec/changes/archive/2026-10-07-add-cost-tracking)

## Context

Budgets need a durable record of every upstream call (answer and judge calls, buffered and
streamed) that survives restarts, without adding database latency to responses. Options: in-memory
only (a restart wipes spend), synchronous database writes (latency and failure on the request
path), Postgres first (heavier for a local-first binary), or SQLite behind an asynchronous writer.

## Decision

Every upstream call becomes one row in a SQLite ledger (WAL mode, `user_version`-guarded schema,
accessed through `sqlx`). Finished entries are queued with `try_send` to a bounded channel; one
background task writes them in batches. In-memory per-key counters (UTC day and month) are updated
at the moment a call completes, so budget enforcement never waits on the database, and are rebuilt
from the ledger at startup. A full queue or a failing database is logged and counted and never
surfaces to a client. Postgres for shared deployments is a later, separate change.

## Consequences

- Entries still queued when the process stops are lost, so spend recomputed after a restart can be
  slightly low. `Ledger::shutdown` drains the queue, but the server does not call it on a signal
  yet (it serves without graceful shutdown), so wiring that is outstanding work.
- Enforcement is by live counters, so in-flight requests can overshoot a limit (bounded and
  documented in `docs/budgets.md`).
- SQL is confined to `src/ledger.rs` and ledger writes never block a response (invariants 3 and 7
  in [invariants.md](../invariants.md)).
- Streams that report no usage are recorded with a missing-usage marker and zero cost.
