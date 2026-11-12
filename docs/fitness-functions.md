# Fitness functions

The checks that keep humpyard's performance and architecture where the [budgets](performance.md) and the
[invariants](invariants.md) say they are, and when each one runs. It is to performance what the
[threat model](threat-model.md) is to security: a register of what can go wrong and what watches for it. The
machine-readable form is `fitness/register.toml`; `scripts/select-checks.py` reads it to pick the checks a change
needs, and `tests/architecture.rs` fails when this page, the register and `src/` disagree.

## Why not run everything every time

The isolation tests take minutes in release mode on a quiet machine, and the absolute latency budgets and the
throughput comparison cannot be run on a shared runner at all: its noise is as large as the tolerance. So a check
runs when a change can move it, every week on `main` (a path map misses a dependency's behaviour and drift made of
many small changes), on demand, and at release. A regression found by the schedule instead of the path map is a
reason to widen that check's triggers, recorded in the register.

## Module classes

Every module under `src/` has a class in the register, and a module without one fails the architecture test.

| Class | Meaning | Modules |
|---|---|---|
| request | On the path of a request | `auth`, `budget`, `clock`, `error`, `estimate`, `health`, `latency`, `limiter`, `metering`, `metrics`, `num`, `policy`, `pool`, `pricing`, `repair`, `repair_request`, `retry_after`, `routing`, `scrub`, `select`, `server`, `signals` |
| shared | Shares something the request path depends on: the runtime setup, the ledger and its writer, the configuration the request path reads, the administrative handlers that read the ledger | `admin`, `config`, `ledger`, `lib`, `main`, `overrides`, `quota` |
| off | Never in the serving path: commands that run once and exit, and the GUI | `ctl`, `discover`, `init`, `migrate`, `prices`, `retired`, `ui`, `ui_assets` |

A request or shared module must be named by the triggers of at least one blocking check, so a module cannot join the
request path without something watching it.

## The checks

| Check | Protects | Runs when | Where | Cost | Blocks |
|---|---|---|---|---|---|
| `architecture-rules` | The invariants, among them 24: heavy ledger reads stay off the runtime and behind the gate | Any change to `src/`, `Cargo.toml`, its own test or this register; weekly; release | Hosted CI, the `check` job | Seconds | Yes |
| `isolation-admin-reads` | [ADR 0036](adr/0036-admin-and-ui-work-must-not-change-the-request-paths-performance.md): administrative reads do not move request latency (p99 within 10% or 1 ms of the unloaded run, on one worker, no entry dropped) | A change to a request or shared module, `Cargo.toml`, `Cargo.lock`, the toolchain file, `tests/isolation.rs` or `tests/common/`; weekly; release | Hosted CI, the `isolation` job | Minutes | Yes |
| `isolation-prune-contention` | A large prune holds the write lock only briefly (slowest override write under 500 ms, requests unaffected) | The same | Hosted CI, the `isolation` job | Minutes | Yes |
| `overhead-budgets` | The [budgets](performance.md): added latency p50 at most 2 ms and p99 at most 10 ms at 64 concurrent, and the streaming first chunk | At release | The reference machine, by hand | Minutes | Yes, at release |
| `memory-budget` | Resident memory under load at most 64 MiB and growth between two loads at most 8 MiB | At release | The reference machine, by hand | Minutes | Yes, at release |
| `throughput-comparison` | Throughput no more than 10% below the previous release, measured alongside it | At release | The reference machine, by hand | Minutes | Yes, at release |
| `ledger-scale` | Ledger reads and writes at 450,000 rows | At release | The reference machine, by hand | Minutes | No |

The weekly run does not claim to cover the reference-machine checks. They are run and recorded at release (see
[Deployment](deployment.md)).

## Adding a module or a check

A new module under `src/` gets a class in `[modules]`. A new check gets an entry in `[[checks]]` and a row here with
the reason it exists; if it is slow, say what can move it and let the register decide when it runs. A change that does
either updates the register in the same pull request, and the architecture test says so when it has not.
