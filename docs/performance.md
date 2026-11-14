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

# Streaming overhead (the same test file; it runs with the one above unless filtered), and the gateway's memory
# under load (runs the real binary as a child process and samples its resident set)
cargo test --release --test overhead gateway_streaming -- --ignored --nocapture
cargo test --release --test memory -- --ignored --nocapture

# Throughput: the current build against the previous release's, in alternating runs (builds that release in a scratch
# directory; add HUMPYARD_BUDGET_ENFORCE=1 to fail on a regression or an inconclusive comparison)
scripts/compare-throughput.sh v0.28.0

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

The ledger starts a prune after the first batch its writer handles after a start, and then hourly, on a task of
its own, so the writer keeps taking messages while it works. A prune deletes in statements of at most 10,000 rows
(each its own transaction) and rests 20 ms between them, so the database's write lock is never held for long and a
writer waiting for it gets in between statements. At steady state each prune removes about a day of rows; the first
prune after a long gap, or after lowering the setting, removes many, in as many batches as it takes (Run 5 shows what
one statement cost, Run 8 what batches cost). A prune cut short, by the gateway stopping, leaves the rest for the next
one.

## The ledger's connections

The ledger opens two pools of two connections each on the same database file (WAL mode). The **writer's pool** serves
the writer task, the override tables, `flush` and shutdown: a write waits at most 5 seconds for the database's
write lock and 5 seconds for a connection. The **read pool** serves statistics, sessions, decisions, quota history
and the other reads: its connections refuse writes (`PRAGMA query_only = ON`) and a read waits at most 1 second
for the lock and 2 seconds for a connection, then fails as busy, so a read never takes a connection the writer needs
and never holds a request for the library defaults of 5 and 30 seconds. Under WAL, readers and the writer do not
block each other; the contention that remains is writer against writer (Run 5). The values are constants of the
ledger module (`WRITER_BUSY_TIMEOUT`, `READ_BUSY_TIMEOUT` and their acquire counterparts), not settings.

## Budgets

What "performance" means for humpyard, and what a change must stay under ([ADR 0036](adr/0036-admin-and-ui-work-must-not-change-the-request-paths-performance.md)).
Administrative, GUI and statistics work must not change the request path, and a change that adds work to the
request path or to a shared resource (the serving runtime, the ledger pool, the writer's queue, the CPU allowance)
carries a measurement. Latency and isolation are budgets on the gateway's own work; throughput is always compared
with the previous release, never with a number written down on another day (Run 3 shows why).

Which checks run for a change is decided by the [fitness register](fitness-functions.md). The result to require on a
pull request is the `performance-gate` job: it always reports, passes when no heavy check was selected, and fails when
one that ran failed. The budgets that need a quiet machine (latency, memory, throughput) are checked by hand at
release, not in CI.

| Measure | Budget | Basis | Checked by | State |
|---|---|---|---|---|
| Gateway-added latency, p50 (against a direct call, 50 ms upstream, up to 64 concurrent requests) | at most 2 ms | [Run 2](#run-2-gateway-overhead-2026-10-18): +0.41 to +1.22 ms; [Run 3](#run-3-the-current-release-against-095-2026-11-09): +0.21 to +1.73 ms | the overhead harness with `HUMPYARD_BUDGET_ENFORCE=1` on a reference machine, a step of the release | met in both runs |
| Gateway-added latency, p99 (same cells) | at most 10 ms | Run 2: +0.59 to +5.72 ms; Run 3: +0.41 to +5.14 ms, and one cell of +16.30 ms | the same | met, except one noisy cell in Run 3 (see its limits) |
| Throughput (64 concurrent requests, instant upstream) | median of current / previous at least 0.90 over three alternating pairs, and neither build's own runs more than 10% apart | [Run 3](#run-3-the-current-release-against-095-2026-11-09) (why it is paired) and [Run 6](#run-6-the-comparison-script-2026-11-10) | `scripts/compare-throughput.sh <previous tag>`, as a step of the release | met in Run 6 against 0.28.0 (median 1.030); inconclusive in Run 3 and in Run 6's comparison of a build with itself (noisy sessions) |
| Request latency under administrative reads (one-worker gateway, 16 concurrent requests) | loaded p99 at most the unloaded p99 plus the larger of 10% and 1 ms; no ledger entry dropped | [Run 4](#run-4-admin-reads-against-the-request-path-2026-11-09) (before the fix) and [Run 9](#run-9-the-fix-on-the-host-and-under-a-cpu-limit-2026-11-11) (after) | the isolation test (CI, the `isolation` job, selected by [what a change can move](fitness-functions.md)) | **passes on the host**: 8.46 ms unloaded, 6.71 ms loaded (it was 9.67 and 34.38 ms). **Fails under a CPU limit of 1 or less** (Run 9): a known gap |
| Streaming: gateway-added time to the first chunk (20 chunks 5 ms apart, up to 64 concurrent streams) | p50 at most 2 ms, p99 at most 10 ms | [Run 7](#run-7-streaming-overhead-and-memory-under-load-2026-11-10): the worst of four runs was +1.00 ms and +3.26 ms | the streaming overhead harness with `HUMPYARD_BUDGET_ENFORCE=1`, a step of the release | met |
| Streaming: gateway-added time to the whole stream (same) | p50 at most 15 ms, p99 at most 30 ms | Run 7: the worst was +9.82 ms and +15.46 ms; the sign flips between runs, so this is a loose bound | the same | met |
| Resident memory of the gateway process under load (64 concurrent requests, plain and streamed) | peak at most 64 MiB | Run 7: 37.1 to 41.3 MiB (the cluster requests 64 MiB for the container) | the memory harness, enforced at release | met |
| Memory growth between two identical loads | at most 8 MiB | Run 7: +1.5 to +3.4 MiB | the same | met |
| Override writes during a large prune | complete within 500 ms (the writer's busy timeout, at which a write fails, is 5 s); request p99 unmoved | [Run 5](#run-5-a-large-prune-against-the-request-path-and-override-writes-2026-11-10) | the write-contention test | passes: the slowest write took 162 to 182 ms after the prune was batched ([Run 8](#run-8-the-same-prune-in-batches-2026-11-11), [Run 9](#run-9-the-fix-on-the-host-and-under-a-cpu-limit-2026-11-11)), against 2.4 s before; the test now fails above 500 ms (the median over its rounds of each round's slowest write, so one stall of a shared disk does not decide it) |

Not budgeted yet: **TLS** (every run here is plain HTTP on loopback; a TLS run needs certificates in the harness
and the number depends on the provider's handshake), **behaviour over a long soak** (Run 7 compares two identical
loads minutes apart, which says nothing about hours of use; a soak is its own run of hours), and **restart time on
a large ledger** (Run 1 measured it, 1.27 s of budget hydration at 1,000,000 rows, but nothing states what is
acceptable and it depends on the deployment). A budget for each is set from its first measurement.

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
fail the write, and the write-ahead log grew to about 100 MB. Prunes in short batches were the fix (Run 8).

Limits: one machine and one ledger size; the writes were made by one client; checkpointing is SQLite's default and
was not tuned.

## Run 6: the comparison script (2026-11-10)

Why: to check `scripts/compare-throughput.sh` on real runs, and to see how often a laptop session is quiet enough
for its verdict. Same machine and toolchain as Run 3. The script builds the previous harness in a scratch
directory, alternates it with the current one for three pairs (previous first in pairs 1 and 3, current first in
pair 2), and reads the gateway's requests per second at 64 concurrent requests against an instant upstream, 5,000
requests per cell. The current build is 0.29.0's gateway code. Commands: `scripts/compare-throughput.sh HEAD`
and `scripts/compare-throughput.sh v0.28.0`.

| Pair | HEAD (previous) | current | ratio | | v0.28.0 (previous) | current | ratio |
|---:|---:|---:|---:|---|---:|---:|---:|
| 1 | 45,188 | 44,057 | 0.975 | | 48,004 | 50,309 | 1.048 |
| 2 | 42,442 | 49,074 | 1.156 | | 49,267 | 50,756 | 1.030 |
| 3 | 49,519 | 51,573 | 1.041 | | 50,368 | 50,784 | 1.008 |
| **median ratio** | | | **1.041** | | | | **1.030** |
| spread within one build | 14.3% | 14.6% | | | 4.7% | 0.9% | |
| **verdict** | | | **inconclusive** | | | | **ok** |

### What it shows

- Against v0.28.0 the current build is level (a median ratio of 1.030, and runs within 5% of each other): no
  regression in the request path since the last release. The figures of about 48,000 to 51,000 req/s are in line
  with Run 2.
- The comparison of a build with itself came out at a median of 1.041, within 10% of 1 as it should, but the
  session was noisy: one build's runs ranged from 42,442 to 49,519 req/s, so the script called it inconclusive,
  which is its purpose. The two invocations were minutes apart on the same machine and the first was far noisier.

### Limits of this run

Three pairs each, one machine, a mock upstream, loopback, no TLS. The spread rule (the range of a build's own runs
within 10% of the largest) is a range statistic, so it grows with the number of runs; whether 10% over three runs
is the right bar for a laptop is open, and a noisy comparison is better repeated than trusted in either direction.

## Run 7: streaming overhead and memory under load (2026-11-10)

Why: the two measures the budgets had no figure for. Same machine and toolchain as Run 3, the gateway code of
0.29.0. Commands: `cargo test --release --test overhead gateway_streaming -- --ignored --nocapture` (four runs,
the last with `HUMPYARD_BUDGET_ENFORCE=1`) and `cargo test --release --test memory -- --ignored --nocapture` (four
runs, the last enforced).

### Streaming

The mock upstream streams 20 chunks 5 ms apart and then a final chunk with the usage; the client reads each stream
to its end. 50 streams per concurrent worker, direct and through the gateway, at concurrency 1, 16 and 64. Direct,
the first chunk arrives in 0.18 to 0.33 ms (p50) and a whole stream takes about 124 to 127 ms (20 chunks, rounded
up by the timer). What the gateway adds, in milliseconds:

| Added to the first chunk, p50 / p99 | run 1 | run 2 | run 3 | run 4 |
|---|---|---|---|---|
| concurrency 1 | +0.61 / +0.75 | +0.36 / +0.72 | +0.48 / +0.63 | +0.72 / +2.90 |
| concurrency 16 | +0.52 / +0.91 | +0.86 / +1.14 | +0.66 / -0.54 | +0.64 / +1.48 |
| concurrency 64 | +0.42 / +3.26 | +1.00 / +2.93 | +0.77 / +1.82 | +0.84 / +2.32 |

| Added to the whole stream, p50 / p99 | run 1 | run 2 | run 3 | run 4 |
|---|---|---|---|---|
| concurrency 1 | +1.53 / -2.84 | +0.53 / +4.98 | -0.01 / -4.35 | +1.44 / -5.44 |
| concurrency 16 | -0.43 / -5.42 | +1.85 / +9.65 | +1.92 / +15.46 | -2.34 / -5.06 |
| concurrency 64 | +9.82 / +12.27 | -4.00 / -2.84 | -4.85 / -6.21 | -0.64 / -0.64 |

The first chunk is reliably cheap: the gateway adds under a millisecond at the median (at most +1.00 ms) and up to
3.3 ms at p99, at every concurrency. The whole-stream figures are noise: they change sign between runs, and the
+9.82 ms of run 1 at concurrency 64 did not come back. A stream's end is decided by 20 timer ticks in the mock, so
this measure resolves about 10 ms; its budget is a loose bound that catches a gateway which makes streams
materially slower, not a precise one.

### Memory

The real `humpyard serve` binary runs as a child process with a ledger on local disk, against a mock upstream in the
test process, and its resident set is read with `ps` every 25 ms. Three loads at 64 concurrent requests: 100,000
plain requests, 3,200 streams, and the 100,000 plain requests again. Each is followed by 2 s of rest.

| MiB resident | run 1 | run 2 | run 3 | run 4 |
|---|---:|---:|---:|---:|
| idle after start | 14.7 | 14.7 | 14.6 | 14.5 |
| after plain requests (peak equals rest) | 37.7 | 37.3 | 37.1 | 37.9 |
| after streams | 38.8 | 38.4 | 38.5 | 39.7 |
| after plain requests again | 39.2 | 39.6 | 39.5 | 41.3 |
| growth between the two plain loads | +1.5 | +2.3 | +2.4 | +3.4 |

The gateway idles at about 15 MiB and peaks at about 40 MiB at 64 concurrent requests, a fraction of the 256 MiB
the cluster allows the container; the process does not return memory to the system between loads (its resident
set at rest equals the peak). Each identical load left it a little higher (+1.5 to +3.4 MiB the second time); the
figure tapers across the phases but two points cannot say whether it plateaus.

### Limits of this run

One machine, a mock upstream, loopback, no TLS. The streaming mock sends small chunks at a fixed gap; real
providers send chunks of uneven size and timing, and long answers have far more than 20. The memory runs measure
only the gateway process, with a small ledger (it grows by the run's own rows), and run for seconds: whether
memory stops growing over hours is the soak, which is not measured.

## Run 8: the same prune, in batches (2026-11-11)

Why: task 2.5 of the performance change cut the decision, override and quota prunes into statements of at most 10,000
rows, with a 20 ms rest between them, run on a task of their own instead of inside the writer. Same machine, toolchain,
test and settings as Run 5 (450,000 usage rows and 300,000 expired decisions, three rounds, a fresh one-worker gateway
per half), after the change: `cargo test --release --test isolation a_large_prune -- --ignored --nocapture`.

| Round | p99, no prune (ms) | p99, pruning (ms) | slowest override write, pruning (ms) | slowest, no prune (ms) | WAL after (bytes) |
|---:|---:|---:|---:|---:|---:|
| 1 | 9.04 | 7.87 | 161 | 6 | 11,865,632 |
| 2 | 9.19 | 7.85 | 152 | 11 | 12,051,032 |
| 3 | 9.20 | 7.82 | 162 | 5 | 12,430,072 |
| **median / max** | **9.19** | **7.85** | **162** | | |

Against Run 5 the slowest override write fell from 1,802 to 2,410 ms to 152 to 162 ms (about fifteen times shorter)
and the write-ahead log from about 100 MB to about 12 MB, with request p99 unmoved as before (9.19 ms against 7.85
ms), no entry dropped and every override write successful. The 150 ms that remains is what a write waits for the
lock between statements: one batch of 10,000 rows plus the busy handler's polling. The test passes at either wait, so
it cannot catch a regression of the lock time; task 2.6 tightens it.

A test of the batching itself (`tests/ledger_prune.rs`, 60,000 expired rows, batches of 2,000) counts the override
writes that complete while the prune is still working: 3 behind one statement and 58 behind batches.

Limits: one machine and one ledger size; the writes were made by one client; checkpointing is SQLite's default (the
smaller write-ahead log is the effect of shorter transactions, not of tuning).

## Run 9: the fix on the host and under a CPU limit (2026-11-11)

Why: task 2.6 of the performance change. With the read pool (2.1), the fold off the runtime (2.2), the gate and shared
result (2.3) and the batched prune (2.5) in place, the isolation tests are run again, the contention test is tightened,
and the one thing CI cannot show is checked by hand: what a container CPU limit does.

### On the host

Apple M5 (10 cores), macOS, `--release`, 0.29.0's gateway code with the changes above. `cargo test --release --test
isolation admin_reads -- --ignored --nocapture` and `... a_large_prune ...`.

| Admin reads, one-worker gateway | unloaded p99 (ms) | loaded p99 (ms) | heavy reads completed | refused |
|---|---:|---:|---:|---:|
| round 1 | 8.46 | 7.02 | 118 | 58,890 |
| round 2 | 8.51 | 6.71 | 296 | 59,409 |
| round 3 | 8.37 | 6.74 | 311 | 58,964 |
| round 4 | 8.41 | 6.44 | 303 | 59,340 |
| round 5 | 11.00 | 6.34 | 313 | 60,277 |
| **median** | **8.46** | **6.71** | | |

The noise check agreed (8.52 against 8.50 ms) and nothing was dropped: the test **passes**, where Run 4 failed it at 9.67
against 34.38 ms. The load asks for a different window every time and ignores `Retry-After`, so what it loads is one
heavy read computing at a time (about 300 a round) and a flood of about 59,000 refusals a round.

| Write contention | request p99, no prune / pruning (ms) | slowest override write (ms) | WAL after (bytes) |
|---|---|---:|---:|
| round 1 | 9.94 / 8.55 | 177 | 13,064,552 |
| round 2 | 10.94 / 8.56 | 182 | 11,371,232 |
| round 3 | 10.38 / 8.52 | 164 | 12,228,192 |

It passes with the limit now 500 ms. To check that the tightened test can fail, the batch size was set to 10,000,000 (a
prune back to one statement) for one run: the slowest write took 1,369 to 1,392 ms, the write-ahead log was 102 to 114
MB, and the test failed naming the 500 ms limit.

### Under a CPU limit

The gateway container in the cluster has a 500m CPU limit, which a thread cannot get around: when a container has used its
quota for a period, the kernel pauses every thread in it. The test binary was built for Linux (Docker Desktop, 10 CPUs,
`rust:1.99-bookworm`) and run with `docker run --cpus N`, the same test and seeding (80,000 usage rows, about 53,333
decisions, past the cap; 2,000 requests per run; three rounds; the gateway on a one-worker runtime):

```sh
docker run --rm -v "$PWD":/src:ro -v hy-target:/target -e CARGO_TARGET_DIR=/target \
  -e RUSTUP_TOOLCHAIN=1.99.1-aarch64-unknown-linux-gnu -w /src rust:1.99-bookworm \
  cargo test --release --test isolation --no-run
docker run --rm --cpus 0.5 -v hy-target:/target:ro -e ISOLATION_ROWS=80000 -e ISOLATION_REQUESTS=2000 \
  -e ISOLATION_ROUNDS=3 rust:1.99-bookworm /target/release/deps/isolation-HASH admin_reads --ignored --nocapture
```

| CPU limit | load | median p99 unloaded (ms) | median p99 loaded (ms) | verdict |
|---:|---|---:|---:|---|
| 4 | loops as fast as it can | 11.39 | 6.71 | pass |
| 2 | loops as fast as it can | 10.59 | 9.92 | pass |
| 1 | loops as fast as it can | 10.34 | **67.31** | fails |
| 0.5 | loops as fast as it can | 10.49 | **91.17** | fails |
| 4 | polite (`ISOLATION_POLITE=1`: waits out `Retry-After`) | 10.78 | 10.83 | pass |
| 0.5 | polite | 10.71 | **43.82** | fails (rounds of 43.82, 10.35 and 67.95 ms) |

At two CPUs or more the heavy reads do not move the request path; at one or less they do, by tens of milliseconds, even
when only two to four heavy reads run in a round (the polite load). Moving the work to another thread cannot help once the
container's quota is spent, because every thread waits.

Limits, which make these figures an overstatement of the cluster's case: the load generator and the mock upstream share the
container's quota with the gateway (in the cluster only the gateway is limited), and the "as fast as it can" load spends
quota on tens of thousands of refusals. The polite run is closer to a GUI, and it still fails at 0.5. One machine, three
rounds, a mock upstream, 16 concurrent requests.

**A known gap, with a cheap mitigation that this change does not make.** The cluster's gateway container asks for 50m and is
limited to 500m of CPU. Raising the limit (not the request) to two CPUs costs nothing while the gateway is idle (it uses
about 1m) and gives a heavy read a second core to burst on, which is what the 2-CPU rows show. Short of that, the options
are the escalation of ADR 0036 (a separate process with its own allowance, reading a local snapshot of the ledger), a
cheaper fold, or a lower row cap.

## Open findings

These are recorded here, not fixed, and all are low priority. Each would be its own change with its own spec,
started on the trigger listed in the [roadmap](roadmap.md#later-and-only-on-a-measured-trigger).

1. **`usage` is never pruned.** Whether it needs a retention setting or an archival command depends on the
   deployment; for a single operator it is not urgent.
2. **`spend_since` cannot use an index on the timestamp.** An index on the timestamp, or a different query shape,
   would make restarts fast on very large ledgers.
3. **The request-id lookup scans.** Rewriting the optional filters so SQLite can use the `request_id` index fixes it.
4. ~~A prune after a long gap blocks writes~~ for as long as it runs (1.3 s for 511,112 rows in Run 1; in Run 5 an
   override write waited 1.8 to 2.4 s behind a prune of 300,000 decisions and the write-ahead log reached about
   100 MB). **Resolved:** prunes run in batches of 10,000 rows with a rest between them, on a task of their own
   ([Run 8](#run-8-the-same-prune-in-batches-2026-11-11)): the slowest override write fell to 162 ms and the
   write-ahead log to about 12 MB.
5. **A CPU limit of one or less lets heavy reads move the request path** (Run 9): p99 rose from about 10 ms to 44 to
   91 ms at 0.5 CPU and 67 ms at 1 CPU, and not at 2 CPUs. The cluster's gateway container is limited to 500m.
   Raising its CPU limit to two is the cheap mitigation; the escalation in ADR 0036 is the structural one.

## Run 10: the isolation tests on hosted runners (2026-11-12)

The isolation job passed on its first pull request (#289) and on four dispatches of `main`, then failed twice on the
pull request that added the series endpoint, a different test each time: the contention test with an override write
that waited 1,174 ms (usually 25 to 135 ms on a runner), and, on the rerun, the admin reads test with a loaded p99 of
12.17 ms against 8.13 ms unloaded. The same change passed locally (one run in four on this laptop also saw a 546 ms
write once, usually 160 to 190 ms), so these were the shared machine, not the code. Two changes: the contention verdict
on the slowest write is the median over its rounds (a prune that holds the lock makes every round slow, as the run with
a prune back in one statement did, 1.37 to 1.39 s in each), and the job runs the tests once more after a failure and passes only if
the second run passes, leaving a warning. This makes a flake less likely to block a pull request; a regression that moves
every run still fails both.

## Run 11: the series in the isolation load (2026-11-12)

The administrative load of the isolation test is now three clients, not two: statistics by target, session views and
a statistics series (`/admin/v1/stats/series`, a different window each time, one-day buckets, 25 groups), each looping
as fast as it can against the one-worker gateway and the ledger seeded past the row cap. All three go through the
admin surface's one gate and the ten-second shared result, so the series is one more heavy read competing for it; the
test now also fails if no series read was answered in any round, so a gate that always served the others first could
not hide it (`add-gui-query-view` 5.3).

Two full runs on the host: loaded p99 6.74 ms against 9.27 and 9.37 ms unloaded (allowed rise 1.00 ms), no entry dropped,
about 300 reads answered and about 90,000 refused per round, none failed. The contention test passed in the same runs
(slowest override write 220 and 291 ms, median of the rounds). The loaded half is again no slower than the unloaded
one, as in Run 9: a heavy read answered from another thread leaves the request path alone, and a refusal costs little.

## Run 12: a key, and an actor on it, in the request traffic (2026-11-13)

Why: `add-actors` puts two new things on every request that carries a key: the lookup now asks whether the key is
revoked (an in-memory scan of the key revocations in force, empty unless one is), and the decision is written with the
actor and owner the key names. Until now no harness sent a key at all, so neither was measured. Both harnesses now take
a mode for how their traffic authenticates, and the isolation test uses `actor` by default, so the CI job exercises the
new path on every run:

- `none`: no keys, an open gateway, as every earlier run;
- `plain`: a client key with no actor;
- `actor`: a client key that names an agent actor owned by a human, so each request goes through the lookup, the
  revocation check and the actor and owner written on its decision.

Same laptop and toolchain as Runs 9 to 11, the gateway code after `add-actors` group 3. Commands:
`ISOLATION_KEY=<mode> cargo test --release --test isolation -- --ignored --nocapture` and
`OVERHEAD_KEY=<mode> cargo test --release --test overhead -- --ignored --nocapture --test-threads=1`, one run of each
mode, the three in the same order back to back.

### Isolation (one worker, the ledger seeded past the row cap, three administrative load clients)

| Request traffic | Unloaded p99 | Loaded p99 | Dropped | Slowest override write (median of the rounds) | p99 with a prune, against without |
|---|---|---|---|---|---|
| `none` | 8.06 ms | 6.75 ms | 0 | 294 ms | 8.32 against 8.26 ms |
| `plain` | 8.24 ms | 6.89 ms | 0 | 157 ms | 8.08 against 8.37 ms |
| `actor` | 8.15 ms | 6.89 ms | 0 | 158 ms | 8.09 against 8.47 ms |

All three pass (allowed rise 1.00 ms); the loaded half is again no slower than the unloaded one, as in Runs 9 and 11,
and every round answered reads, series included. The unloaded p99 of the three modes differs by less than the 0.3 ms
that two halves of one mode differ by in the noise check (7.92 against 7.87, 7.92 against 7.96 and 8.31 against 8.95 ms).

### Gateway overhead (the 50 ms upstream, and the instant one that isolates the gateway's own work)

| Added by the gateway, p50 / p99 (ms) | `none` | `plain` | `actor` |
|---|---|---|---|
| instant upstream, concurrency 1 | +0.06 / +0.14 | +0.06 / +0.08 | +0.06 / +0.11 |
| instant upstream, concurrency 16 | +0.22 / +0.30 | +0.20 / +0.26 | +0.20 / +0.25 |
| instant upstream, concurrency 64 | +0.86 / +1.27 | +0.85 / +1.15 | +0.86 / +1.23 |
| 50 ms upstream, concurrency 64 | +1.15 / +3.00 | +0.94 / +2.85 | +1.34 / +3.69 |
| instant upstream, concurrency 64, gateway requests per second | 48,816 | 48,815 | 48,533 |

The gateway's own work, with no upstream delay to hide it, is the same to a hundredth of a millisecond at the median in
the three modes, and its throughput differs by 0.6%. The 50 ms rows move by a few tenths of a millisecond in no
particular direction, which is the noise of a timer-driven mock (Run 7). Every budget is met in every mode, with the
worst cases at concurrency 64 or 1: added p50 at most 1.34 ms (limit 2) and p99 at most 3.69 ms (limit 10) on the
50 ms upstream; first chunk p50 at most 0.72 ms and p99 at most 3.11 ms; whole stream p50 at most 1.03 ms (limit 15) and
p99 at most 9.44 ms (limit 30).

What this does and does not show: a key and an actor add less to a request than one run on this machine can resolve, and
nothing moved a budget. It is one run per mode, not the alternating comparison of Run 6, and it is not the
throughput-against-the-previous-release check, which belongs to a release. The check that blocks a merge is the
isolation job (the performance gate), which runs on the pull request that carries this run.
