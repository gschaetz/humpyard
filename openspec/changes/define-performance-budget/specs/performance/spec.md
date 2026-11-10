## ADDED Requirements

### Requirement: Performance has recorded budgets
`docs/performance.md` SHALL state each budget with its measure, value, basis and how it is checked, SHALL include for gateway-added latency against a direct call (50 ms upstream, up to 64 concurrent requests) a p50 of at most 2 ms and a p99 of at most 10 ms, and for throughput at most 10% below the recorded baseline on the same machine, and SHALL list the measures that have no budget yet and why.

#### Scenario: A budget has a basis
- **WHEN** the Budgets section is read
- **THEN** each budget names the recorded run it was set from and whether CI or the release check enforces it

#### Scenario: An unmeasured measure
- **WHEN** a measure such as memory under load, streaming overhead, TLS or soak has not been measured
- **THEN** the page lists it as unbudgeted and says a measurement run comes first

### Requirement: Absolute budgets are checked at release on a reference machine
The overhead harness SHALL compare its results with the absolute budgets and report each as met or exceeded, SHALL fail only when run with budget enforcement switched on, and the release procedure SHALL run it that way on a reference machine and record the result in `docs/performance.md`.

#### Scenario: Default run reports only
- **WHEN** the harness is run without enforcement and a budget is exceeded
- **THEN** it reports the budget as exceeded and exits successfully

#### Scenario: Enforced run
- **WHEN** the harness is run with enforcement and a budget is exceeded
- **THEN** it fails and names the budget and the measured value

### Requirement: Admin load must not move request latency
The repository SHALL contain an isolation test, run in CI in release mode, that runs the overhead harness against a gateway on a runtime with one worker thread, with and without looping administrative statistics reads against a ledger seeded past the statistics row cap, and SHALL fail when the request p99 with load is more than 10% or 1 millisecond higher (whichever is larger) than without, or when the ledger dropped any entry.

#### Scenario: No load, no difference
- **WHEN** the test runs with the admin load switched off in both halves
- **THEN** it passes, so the threshold is not tripped by noise alone

#### Scenario: The load cannot be shed away
- **WHEN** the admin reads are all refused as busy
- **THEN** the test fails for having made no successful read, so a gate that rejects everything does not pass it

#### Scenario: Write contention
- **WHEN** a large prune and an override write happen during a loaded run
- **THEN** the test fails if request p99 moves past the threshold, an entry is dropped, or the override write fails or exceeds the writer's busy timeout, and it records the size of the write-ahead log

#### Scenario: A regression is caught
- **WHEN** a statistics read is changed to run its row fold on the serving runtime
- **THEN** the test fails on the p99 threshold

### Requirement: A change to the request path or a shared resource carries a measurement
A change that adds work to the request path, or uses the serving runtime, the ledger pool, the writer's queue or the process's CPU allowance, SHALL include a performance task that records a before and after measurement or passes the isolation test, and `openspec/config.yaml` SHALL require this of every such change's tasks.

#### Scenario: A task is missing
- **WHEN** a change's design names the serving runtime or the ledger pool as used and its tasks hold no performance task
- **THEN** the change is not ready to apply
