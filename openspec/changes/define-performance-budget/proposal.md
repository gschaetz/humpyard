# Proposal

## Why

humpyard records what the gateway costs (`docs/performance.md`, two harnesses) but states no target a change must stay under, and nothing in CI checks one. The admin API and the GUI are arriving, and they read the same ledger and run on the same runtime as the request path. The code already shows how that can go wrong: the statistics read folds up to 50,000 decision rows' JSON inline on the serving runtime without yielding, through a ledger pool of two connections shared with the usage writer, and the writer drops entries when its queue fills. On the cluster the gateway container has a 500m CPU limit, which may leave the runtime one worker thread. None of this is measured today. The author's own traffic is tiny, so the risk is the precedent: each new admin or UI feature is another reader of the same resources, and without a budget and a test a regression is only found by a user.

## What Changes

- **Performance gets targets.** A Budgets section in `docs/performance.md` defines the measures and their initial values: gateway-added latency against a direct call (p50 at most 2 ms and p99 at most 10 ms with a 50 ms upstream at up to 64 concurrent requests) and throughput no more than 10% below the previous release's, measured alongside it on the same machine in one session (a recorded figure from another day is as noisy as the tolerance, as a check in this change showed). Memory under load, streaming overhead, TLS and soak are unmeasured, so a measurement run comes first and their budgets are set from it.
- **A relative isolation test fails CI.** It runs the overhead harness against a gateway on a one-worker runtime while a ledger seeded past the row cap serves looping statistics reads, and fails if request p99 rises by more than 10% or 1 ms (whichever is larger) over the same run without admin load, or if the ledger drops an entry. It is written first and shown failing against today's code. Absolute latency budgets and the paired throughput comparison are checked on a reference machine at release time, not in CI, because shared runners are too noisy for them.
- **Heavy reads are isolated from the request path.** The row fold runs off the serving runtime; reads use a read-only pool of their own and never take the writer's connections; at most one heavy read runs at a time per surface (admin and client) and a second is answered 503 with `Retry-After` instead of queueing; identical reads within ten seconds share one computation. Statistics may therefore be up to ten seconds old.
- **Database waits are stated, and long write locks are cut short.** The writer pool's busy timeout and the read pool's busy and acquire timeouts are set explicitly (the library defaults are 5 s and 30 s), reads fail fast with a 503 instead of hanging, and the decision, override and quota prunes delete in batches so no single statement holds the write lock near the timeout. A second isolation test covers a prune and an override write under load and records the size of the write-ahead log.
- **Checks run by risk and by schedule, from a register.** A fitness-function register (`fitness/register.toml`, read as `docs/fitness-functions.md`) says for each performance or architecture check what it protects, what can move it, when it runs and what it costs, like the threat model does for threats. Every module under `src/` gets a performance class (request path, shared resource or off path) and a test refuses an unclassified one, so a heavy check runs when a change can move it, weekly on `main`, on demand and at release, not on every pull request. One always-running gate job is the required check, so a skipped check never leaves a pull request waiting, and a failed weekly run opens (or comments on) a `performance-regression` issue.
- **The rule is enforced, not remembered.** A numbered invariant in `docs/invariants.md` with an architecture test: the ledger exposes only async methods that dispatch heavy reads off the runtime, and the row-fold function is private, so calling it from request-serving code fails the build. `openspec/config.yaml` gains a rule that a change touching the request path or a shared resource carries a performance task and a measurement.
- **The escalation is named.** If the isolation test still fails after these layers, heavy reads move to a separate process working from a local snapshot of the ledger (SQLite backup API or `VACUUM INTO`), never a live replica of the WAL ledger, which the cluster keeps on NFS where WAL's shared memory is unsupported. That would be its own ADR. ADR 0036 records the rule and this path.

Not in this change: stored rollups (ADR 0035 chose against them), a read replica, any change to the request path itself, and the series endpoint (that is `add-gui-query-view`, which gets a task to pass this test).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `performance`: budgets are defined and recorded, the isolation test exists, fails CI when a change can move it (and weekly and at release) and is shown against a baseline, every check is registered with what can move it and a cadence, every module has a performance class, and a change to the request path or a shared resource carries a measurement.
- `admin-api`: heavy admin reads are limited to one at a time, shed with 503 `admin_busy` and `Retry-After`, and identical reads within ten seconds share one result.
- `routing-statistics`: the same limit and sharing apply to the client statistics reads (503 `stats_busy`), and a statistics answer may be up to ten seconds old.
- `usage-ledger`: reads for statistics and sessions run off the serving runtime on a read-only pool of their own, never use the writer's connections, fail fast when they cannot start, and cannot cause an entry to be dropped; prunes hold the write lock only briefly.

## Impact

- `src/ledger.rs` and `src/ledger/stats.rs`, `src/ledger/sessions.rs` (a read pool, its timeouts, blocking dispatch, a private fold), `src/ledger/decisions.rs`, `overrides.rs` and `quota.rs` (batched prunes), `src/server/stats.rs`, `src/server/sessions.rs`, `src/server/admin.rs` (the gates, the cache, the 503), and the new gate and cache module.
- `tests/`: a shared seeding helper (moved from the scale harness), the isolation test, and architecture rules. `fitness/register.toml`, `docs/fitness-functions.md` and `scripts/select-checks.py`: the register and the selector. `.github/workflows/ci.yml`: a `changes` job, the isolation job in release mode run only when selected (and weekly, on dispatch and on a release tag) and an always-running `performance-gate` job as the required check.
- Docs: `docs/performance.md` (Budgets, the isolation result, the release check), `docs/invariants.md`, `docs/architecture.md`, `docs/api.md` (503 `admin_busy` and `stats_busy`, the ten-second freshness), `docs/deployment.md` (the release check), and the threat model page.
- Behavior change for callers: a statistics read can be answered 503 when another heavy read is running, and its numbers can be up to ten seconds old. Request traffic, routing and the ledger schema do not change.
