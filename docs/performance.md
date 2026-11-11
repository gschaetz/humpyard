# Performance and scale

Every performance figure about humpyard should be traceable to a run recorded on this page. Nothing here is a
capacity guarantee: each run states what it measured and what it did not. The harnesses are in the repository and
are not part of the default test run:

```sh
# Ledger scale: LEDGER_SCALE_ROWS usage rows (default 1,000,000) over LEDGER_SCALE_DAYS days (default 30)
LEDGER_SCALE_ROWS=1000000 cargo test --release --test ledger_scale -- --ignored --nocapture

# Gateway overhead: direct to a mock upstream versus through the gateway
cargo test --release --test overhead -- --ignored --nocapture
# ... and fail on an exceeded latency budget (the release check; see Budgets)
HUMPYARD_BUDGET_ENFORCE=1 cargo test --release --test overhead -- --ignored --nocapture

# Isolation: do administrative reads, or a large prune, move the request path? (a one-worker gateway)
cargo test --release --test isolation -- --ignored --nocapture
```

The isolation tests take `ISOLATION_WORKERS` (the gateway's worker threads, default 1), `ISOLATION_ROWS`,
`ISOLATION_EXPIRED_ROWS`, `ISOLATION_REQUESTS` and `ISOLATION_ROUNDS` to change their size; the recorded verdicts
use the defaults.

## What the ledger keeps, and what it prunes

| Table | Holds | Pruned? | Setting |
|---|---|---|---|
| `usage` | one row per upstream call: tokens, cost, outcome (the spend history budgets are computed from) | **No**, kept forever | none |
| `decisions` | one row per request: routing, attempts, outcome (read by `/v1/stats`, `explain`, the decision log) | Yes | `decision_retention_days` (default 90, 0 keeps everything) |
| `quota_samples` | provider quota readings | Yes, with the decisions | `decision_retention_days` |

The pruning runs inside the ledger's writer, so while it deletes, queued writes wait. At steady state each
prune removes about a day of rows and is short; the first prune after a long gap, or after lowering the setting,
removes many at once (see the run below).

## Budgets

What "performance" means for humpyard, and what a change must stay under ([ADR 0036](adr/0036-admin-and-ui-work-must-not-change-the-request-paths-performance.md)).
Administrative, GUI and statistics work must not change the request path, and a change that adds work to the
request path or to a shared resource (the serving runtime, the ledger pool, the writer's queue, the CPU allowance)
carries a measurement. Latency and isolation are budgets on the gateway's own work; throughput is always compared
with the previous release, never with a number written down on another day (Run 3 shows why).

| Measure | Budget | Basis | Checked by | State |
|---|---|---|---|---|
| Gateway-added latency, p50 (against a direct call, 50 ms upstream, up to 64 concurrent requests) | at most 2 ms | [Run 2](#run-2-gateway-overhead-2026-10-18): +0.41 to +1.22 ms; [Run 3](#run-3-the-current-release-against-095-2026-11-09): +0.21 to +1.73 ms | the overhead harness with `HUMPYARD_BUDGET_ENFORCE=1` on a reference machine, a step of the release | met in both runs |
| Gateway-added latency, p99 (same cells) | at most 10 ms | Run 2: +0.59 to +5.72 ms; Run 3: +0.41 to +5.14 ms, and one cell of +16.30 ms | the same | met, except one noisy cell in Run 3 (see its limits) |
| Throughput (64 concurrent requests, instant upstream) | at least 90% of the previous release's, measured alongside it | Run 3 (45,667 and 40,980 req/s against 47,787 and 44,560 for 0.9.5) | a paired comparison with the previous release's binary (`scripts/compare-throughput.sh`, not yet written) | inconclusive in Run 3: two pairs, and one build's own pairs differed by 10.3% |
| Request latency under administrative reads (one-worker gateway, 16 concurrent requests) | loaded p99 at most the unloaded p99 plus the larger of 10% and 1 ms; no ledger entry dropped | [Run 4](#run-4-admin-reads-against-the-request-path-2026-11-09) | the isolation test (it will fail CI once the job is added, which needs the fix first) | **fails today**: 9.67 ms to 34.38 ms |
| Override writes during a large prune | complete within the writer's busy timeout (5 s); request p99 unmoved | [Run 5](#run-5-a-large-prune-against-the-request-path-and-override-writes-2026-11-10) | the write-contention test | passes, but the slowest write took 2.4 s |

Not budgeted yet, because they have not been measured: **memory under load**, **streaming overhead** (both
planned as the next run), **TLS** (every run here is plain HTTP on loopback), **behaviour over a long soak**, and
**restart time on a large ledger** (Run 1 measured it, 1.27 s of budget hydration at 1,000,000 rows, but nothing
states what is acceptable). A budget for each is set from its first measurement.

## Run 1: the ledger at 100,000 and 1,000,000 usage rows (2026-10-18)

humpyard 0.9.5, Apple M5 (10 cores, 16 GiB), macOS 26.6, rustc 1.99.1, local SSD, `--release`, seed 1. Rows are
synthetic and chronological over 30 days: 30 keys, 8 providers, 6 routes, 70 percent of rows with a session id,
85 percent `ok` outcomes. A decision accompanies two of every three usage rows, and one quota sample per 1,000.
The fill goes through the real queue and writer, so it is also the sustained-write measurement.

| | 100,000 rows | 1,000,000 rows |
|---|---:|---:|
| Rows written (usage / decisions / quota) | 100,000 / 66,667 / 100 | 1,000,000 / 666,667 / 1,000 |
| Sustained write, usage + decision rows per second | 46,380 | 35,500 |
| Database size (db + wal) | 55.7 MiB | 523.9 MiB |
| Reopen the ledger | 0.8 ms | 1.3 ms |
| `spend_since`, last day | 2.8 ms | 203.5 ms |
| `spend_since`, whole 30-day window (budget hydration at start-up) | 45.3 ms | 1,267.8 ms |
| `/v1/stats` by route, last day | 8.9 ms | 88.1 ms |
| `/v1/stats` by route, whole window | 144.0 ms | 483.6 ms |
| `/v1/stats` by target / endpoint, whole window | 149.8 / 158.4 ms | 507.0 / 577.0 ms |
| Session scan, last day / whole window | 9.9 / 160.1 ms | 114.3 / 885.9 ms |
| Decisions, newest 100 | 0.5 ms | 0.6 ms |
| Decisions, newest 100 of one route | 1.1 ms | 1.2 ms |
| Decisions, one request id | 10.8 ms | 308.3 ms |
| Prune decisions older than 7 days | 72.7 ms (51,112 rows) | 1,283.2 ms (511,112 rows) |

### What it shows

- **Writes are far faster than any real need.** 35,000 to 46,000 rows per second against the single writer; the
  deployment this was built for records a few hundred rows a day.
- **Two queries do not scale the way an index would suggest.**
  - `spend_since` filters on the timestamp alone, but the only index that contains it is `(key_id, ts_ms)`, so it
    cannot seek; its cost grows with the whole table (a one-day window took 72 times longer at 10 times the
    rows). It runs once, at start-up, to rebuild budget counters, so the cost is a slower restart, not a slower
    request.
  - The decisions lookup by request id uses an optional-filter pattern (`? IS NULL OR request_id = ?`) that stops
    SQLite from using the `request_id` index (308 ms at 1M rows). It is a debugging query, not on the request path.
- **`/v1/stats` and the session view are bounded by design.** They read at most 50,000 rows (the response says
  when it truncated), so their cost stops growing once a window holds more than that.
- **The `usage` table is the one that grows without limit.** It is never pruned.
- **How far away is a million rows?** On the author's gateway, measured on 2026-10-18 from `/v1/stats`: 376
  usage rows in a day (a busy day, with testing), which is about 2,700 days to reach 1,000,000. A team sending
  a thousand times that traffic would reach it in about three days of the same rate, so this matters only
  for shared deployments.

### Limits of this run

One machine and one local disk, synthetic rows, one pass per measurement (not averaged), SQLite's page cache warm
from the fill. It did not measure disk-full behavior, concurrent readers during writes, or a cold start after
a reboot. It says nothing about a different filesystem (network storage is not supported for the ledger).

## Run 2: gateway overhead (2026-10-18)

Same machine and version. The mock upstream returns a fixed completion and the gateway has a ledger on local
disk, so each gateway request includes its ledger write. The client, the mock and the gateway share one process
and one multi-threaded runtime on loopback, so the client and the mock take CPU from the gateway too; this
overstates the gateway's cost under load more than it understates it. Latencies are milliseconds; "overhead" is the
gateway figure minus the direct figure.

| Upstream | Concurrency | Path | p50 | p90 | p99 | req/s | Overhead p50 / p99 |
|---|---:|---|---:|---:|---:|---:|---|
| instant | 1 | direct | 0.02 | 0.03 | 0.05 | 37,944 | |
| instant | 1 | gateway | 0.09 | 0.10 | 0.14 | 10,853 | +0.07 / +0.09 |
| instant | 16 | direct | 0.12 | 0.17 | 0.22 | 128,308 | |
| instant | 16 | gateway | 0.34 | 0.41 | 0.49 | 46,466 | +0.22 / +0.27 |
| instant | 64 | direct | 0.44 | 0.60 | 0.92 | 138,959 | |
| instant | 64 | gateway | 1.29 | 1.62 | 2.10 | 48,437 | +0.85 / +1.18 |
| 50 ms | 1 | direct | 52.28 | 52.46 | 52.92 | 19 | |
| 50 ms | 1 | gateway | 52.85 | 53.27 | 53.51 | 19 | +0.57 / +0.59 |
| 50 ms | 16 | direct | 52.40 | 52.72 | 53.57 | 306 | |
| 50 ms | 16 | gateway | 52.81 | 53.66 | 54.21 | 303 | +0.41 / +0.64 |
| 50 ms | 64 | direct | 52.41 | 53.47 | 54.15 | 1,219 | |
| 50 ms | 64 | gateway | 53.64 | 55.57 | 59.88 | 1,188 | +1.22 / +5.72 |

### What it shows

- Against an upstream that takes 50 ms, the gateway adds well under a millisecond at the median up to 16 concurrent
  requests and about a millisecond and a quarter at 64. Real providers take hundreds of milliseconds to seconds, so
  the gateway's own time is not what a client waits for.
- Throughput through the gateway on this machine was above 46,000 requests per second against an instant upstream,
  far beyond what any provider lets one operator send. **Capacity is not a reason to run more than one replica**
  for a deployment like this; availability during deploys is.

### Limits of this run

One pass per cell; a repeat run of the instant-upstream cells gave +0.07 ms at concurrency 1 and +0.92 ms at 64
(the table says +0.85 ms), so read the figures as good to a tenth of a millisecond or so at the median. A mock upstream, plain HTTP on loopback (no TLS), non-streaming responses, one route and one target, no failover,
no tool or image content, and the single-process contention described above. Streaming overhead, TLS, large
bodies, memory use under load and behavior over a long soak were not measured.

## Run 3: the current release against 0.9.5 (2026-11-09)

Why: to see whether the figures of Run 2 still hold after 19 releases, and how much two runs of one binary differ.
Apple M5 (10 cores, 16 GiB), macOS 27.2, rustc 1.99.1, local SSD, `--release`. "Old" is commit `1e7e1fe` (the
version-0.9.5 tree where Run 2 was recorded; the `v0.9.5` tag predates the harness), "new" is 0.29.0. Each binary's
overhead harness was run with the default 5,000 requests, alternating old, new, old, new, in one session, with
`cargo test --release --test overhead -- --ignored --nocapture`.

| Gateway req/s, instant upstream | old, run 1 | new, run 1 | old, run 2 | new, run 2 | new / old (pair 1, pair 2) |
|---|---:|---:|---:|---:|---|
| concurrency 1 | 11,026 | 11,126 | 11,255 | 11,183 | 1.01, 0.99 |
| concurrency 16 | 46,660 | 47,719 | 37,411 | 38,985 | 1.02, 1.04 |
| concurrency 64 | 47,787 | 45,667 | 44,560 | 40,980 | 0.96, 0.92 |

| Added by the gateway, 50 ms upstream, concurrency 64 | old, run 1 | new, run 1 | old, run 2 | new, run 2 |
|---|---:|---:|---:|---:|
| p50 (ms) | +1.73 | +1.30 | +0.77 | +1.09 |
| p99 (ms) | +5.14 | +3.86 | +4.30 | +2.51 |

### What it shows

- The added latency is the same in both builds within the noise, and the gateway is level at concurrency 1 and 16.
  At concurrency 64 the current build's throughput was 4 to 8% below 0.9.5's in the same session: inside a 10%
  tolerance, but not by much.
- **The same binary varies as much as the tolerance.** Between its own two runs the old build changed by 6.8%
  (47,787 to 44,560 req/s) and the new by 10.3% (45,667 to 40,980), and the second pair was lower for both, which
  looks like the machine, not the code. A single run compared with a figure recorded on another day cannot resolve
  a 10% tolerance. That is why the throughput budget is a paired comparison with the previous release.
- A shortened run made while this was being planned (1,500 requests, with other work running on the machine)
  measured about 40,000 req/s at concurrency 64 and read as a 15% regression from Run 2's figure. It was noise; it
  is not a recorded run.

### Limits of this run

Two pairs, where the comparison procedure asks for three; by its own rule (a build's pairs within 10% of each
other) this comparison is inconclusive. One cell is far above the rest: the old build's p99 at concurrency 16
against the 50 ms upstream added +16.30 ms in its second run (the other three runs gave +1.29, +1.79 and +3.54
ms), so a latency budget judged on one run's worst p99 cell can fail on noise; the release check should repeat a
failing cell before concluding. A mock upstream, loopback, no TLS, non-streaming, one route and one target.

## Run 4: admin reads against the request path (2026-11-09)

Why: ADR 0036 asks that administrative reads never change the request path. The gateway's statistics read folds up
to 50,000 decision rows on the serving runtime, and the cluster gateway container has a 500m CPU limit, which may
leave its runtime one worker thread. This is the baseline against the code of 0.29.0 (the gateway is unchanged
since the release). Same machine and toolchain as Run 3. `cargo test --release --test isolation
admin_reads -- --ignored --nocapture`.

The gateway runs on a tokio runtime with one worker thread, in a thread of its own, over a ledger seeded with
100,000 usage rows and about 66,666 decisions, past the statistics row cap. Request traffic is 16 concurrent
requests against a 5 ms mock upstream, 4,000 requests per run. The load is two clients reading statistics (by
target) and session views back to back, each read for a different window. Five alternating rounds, unloaded then
loaded; before them an unloaded-against-unloaded check (9.48 ms against 9.50 ms) showed the machine quiet.

| Round | unloaded p99 (ms) | loaded p99 (ms) | admin reads completed |
|---:|---:|---:|---:|
| 1 | 9.31 | 34.85 | 24 |
| 2 | 9.67 | 34.38 | 24 |
| 3 | 9.57 | 35.37 | 24 |
| 4 | 9.83 | 32.63 | 24 |
| 5 | 9.67 | 33.58 | 24 |
| **median** | **9.67** | **34.38** | |

The allowed rise was 1.00 ms, so **the test fails**: request p99 rose 3.6 times. No ledger entry was dropped, and
no read was refused or failed. The same run with `ISOLATION_WORKERS=4` shows no effect: median p99 9.14 ms
unloaded and 8.22 ms loaded (rounds of 8.20 to 8.38 ms), with a quiet check of 8.92 ms against 8.96 ms. The
stall belongs to a runtime with few workers.

Limits: one machine, a mock upstream, one seed. Whether the cluster's runtime really has one worker is not
confirmed: it is inferred from the container's CPU limit. CPU-quota throttling, which threads cannot remove, is not
reproduced here and needs a manual run under `docker run --cpus 0.5`.

## Run 5: a large prune against the request path and override writes (2026-11-10)

Why: the ledger's decision prune is one `DELETE`, and the pool has two connections shared by the writer and every
read. Same machine, toolchain and code as Run 4. `cargo test --release --test isolation a_large_prune --
--ignored --nocapture`.

A ledger of 450,000 usage rows and 300,000 decisions, all older than the retention, is copied per half. Each half
starts a fresh one-worker gateway (the ledger prunes after the first batch of writes after a start): one with
retention 0 (nothing to prune) and one with 7 days (all 300,000 decisions are deleted). During 4,000 requests
(16 concurrent, 5 ms upstream) an override is created and removed every 50 ms through the admin API; it names a
target no route uses. Three rounds.

| Round | p99, no prune (ms) | p99, pruning (ms) | slowest override write, pruning (ms) | slowest, no prune (ms) | WAL after (bytes) |
|---:|---:|---:|---:|---:|---:|
| 1 | 9.25 | 11.31 | 1,802 | 5 | 100,717,552 |
| 2 | 9.28 | 9.37 | 2,211 | 5 | 100,779,352 |
| 3 | 9.57 | 8.99 | 2,410 | 20 | 100,779,352 |
| **median / max** | **9.28** | **9.37** | **2,410** | | |

The test passes: request p99 is unmoved by the prune, nothing was dropped and every override write succeeded
inside the writer's 5 s busy timeout. **But an override write waited 1.8 to 2.4 seconds** behind the single long
delete (against 5 to 20 ms with nothing to prune), so a ledger about twice as large would reach the timeout and
fail the write, and the write-ahead log grew to about 100 MB. Prunes in short batches are the planned fix.

Limits: one machine and one ledger size; the writes were made by one client; checkpointing is SQLite's default and
was not tuned.

## Open findings

These are recorded here, not fixed, and all are low priority. Each would be its own change with its own spec,
started on the trigger listed in the [roadmap](roadmap.md#later-and-only-on-a-measured-trigger).

1. **`usage` is never pruned.** Whether it needs a retention setting or an archival command depends on the
   deployment; for a single operator it is not urgent.
2. **`spend_since` cannot use an index on the timestamp.** An index on the timestamp, or a different query shape,
   would make restarts fast on very large ledgers.
3. **The request-id lookup scans.** Rewriting the optional filters so SQLite can use the `request_id` index fixes it.
4. **A prune after a long gap blocks writes** for as long as it runs (1.3 s for 511,112 rows here). In Run 5 an
   override write waited 1.8 to 2.4 s behind a prune of 300,000 decisions and the write-ahead log reached about
   100 MB; the planned fix is to prune in short batches.
