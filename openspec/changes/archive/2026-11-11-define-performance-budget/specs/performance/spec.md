## ADDED Requirements

### Requirement: Performance has recorded budgets
`docs/performance.md` SHALL state each budget with its measure, value, basis and how it is checked, SHALL include for gateway-added latency against a direct call (50 ms upstream, up to 64 concurrent requests) a p50 of at most 2 ms and a p99 of at most 10 ms, and for throughput at most 10% below the previous release's measured alongside it, and SHALL list the measures that have no budget yet and why.

#### Scenario: A budget has a basis
- **WHEN** the Budgets section is read
- **THEN** each budget names the recorded run it was set from and whether CI or the release check enforces it

#### Scenario: An unmeasured measure
- **WHEN** a measure such as memory under load, streaming overhead, TLS or soak has not been measured
- **THEN** the page lists it as unbudgeted and says a measurement run comes first

### Requirement: Absolute budgets are checked at release on a reference machine
The overhead harness SHALL compare its results with the absolute latency budgets and report each as met or exceeded, SHALL fail only when run with budget enforcement switched on, and the release procedure SHALL run it that way on a reference machine and record the result in `docs/performance.md`.

#### Scenario: Default run reports only
- **WHEN** the harness is run without enforcement and a budget is exceeded
- **THEN** it reports the budget as exceeded and exits successfully

#### Scenario: Enforced run
- **WHEN** the harness is run with enforcement and a budget is exceeded
- **THEN** it fails and names the budget and the measured value

### Requirement: Throughput is compared with the previous release in the same session
The repository SHALL contain a procedure that runs the overhead harness of the previous release and of the current build alternately for at least three pairs on one machine, reports each pair and the median ratio of gateway throughput at 64 concurrent requests against an instant upstream, and, with budget enforcement switched on, fails when the median ratio is below 0.90 or when the pairs of one build differ from each other by more than 10%.

#### Scenario: A regression
- **WHEN** the current build's median throughput is 12% below the previous release's
- **THEN** the procedure reports the ratio and, when enforcing, fails

#### Scenario: A noisy machine
- **WHEN** the throughput of one build varies by more than 10% between its own pairs
- **THEN** the procedure reports the comparison as inconclusive and, when enforcing, fails instead of passing

#### Scenario: The same build twice
- **WHEN** the procedure compares a build with itself
- **THEN** the median ratio is within 10% of 1, which is the noise floor the comparison is read against

### Requirement: Admin load must not move request latency
The repository SHALL contain an isolation test, run by CI in release mode under the selection rules, that measures a one-worker gateway with and without looping administrative statistics reads over a ledger seeded past the row cap, and SHALL fail when the request p99 with load is more than 10% or 1 millisecond (whichever is larger) above the p99 without it, or when the ledger dropped any entry.

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

### Requirement: Each check is registered with what can move it
`fitness/register.toml` SHALL list every performance and architecture check with what it protects, the module classes and paths that trigger it, its cadence (on a change, weekly, at release), where it runs (hosted CI or a reference machine) and whether it blocks, and `docs/fitness-functions.md` SHALL present the same checks.

#### Scenario: A check with no trigger and no cadence
- **WHEN** a registered check names no module class, no path and no scheduled cadence
- **THEN** the register's test fails, because nothing would ever run it

#### Scenario: A check that cannot run on a hosted runner
- **WHEN** the absolute latency budgets and the throughput comparison are listed
- **THEN** they are marked as running on a reference machine at release, and the weekly CI run does not claim to cover them

#### Scenario: The page and the register agree
- **WHEN** `docs/fitness-functions.md` lists a check the register lacks, or the reverse
- **THEN** a test fails

### Requirement: Every module has a performance class
`tests/architecture.rs` SHALL fail when a module under `src/` is missing from the register's module table or the table names a module that does not exist, each module being classed as request path, shared resource or off path, and SHALL fail when a request-path or shared-resource module is covered by no blocking check's triggers.

#### Scenario: A new module
- **WHEN** a module is added under `src/` without a class
- **THEN** the architecture test fails until it is classed

#### Scenario: A stale entry
- **WHEN** a module is removed and still has a class in the register
- **THEN** the architecture test fails

#### Scenario: A request-path module nothing watches
- **WHEN** a module is classed as request path and no blocking check lists its class or its files as a trigger
- **THEN** the architecture test fails

### Requirement: Checks run when a change can move them, on a schedule and at release
CI SHALL select the registered checks a pull request triggers from the files it changes, SHALL run the selected ones, SHALL run every registered hosted-CI check weekly on `main`, on manual dispatch and on a release tag, and SHALL report one gate result that passes when no check was selected or every selected check passed and fails when a selected check failed or was cancelled.

#### Scenario: A documentation-only change
- **WHEN** a pull request changes only files under `docs/` and `openspec/`
- **THEN** no heavy check runs and the gate passes

#### Scenario: A change to a request-path module
- **WHEN** a pull request changes a file of a module classed as request path
- **THEN** the isolation check runs, and the gate fails if it fails

#### Scenario: A dependency or toolchain change
- **WHEN** a pull request changes `Cargo.lock`, `Cargo.toml` or the toolchain file
- **THEN** every check that lists them as a trigger runs, because a dependency can move any of them

#### Scenario: The schedule and the release
- **WHEN** the weekly run, a manual dispatch or a release tag starts the workflow
- **THEN** every registered hosted-CI check runs whatever files changed

#### Scenario: A skipped check does not block the pull request
- **WHEN** a heavy job is skipped because nothing selected it
- **THEN** the required gate still reports a result, so the pull request is not left waiting

### Requirement: A failed weekly run is reported as an issue
When the weekly run of the registered checks fails, CI SHALL open an issue labelled `performance-regression` that names the failed checks and the run, or add a comment to the open one, SHALL NOT close it, and SHALL NOT open issues for pull-request runs or manual dispatches.

#### Scenario: The first failure
- **WHEN** the weekly run fails and no `performance-regression` issue is open
- **THEN** an issue is opened that names each failed check and links the run

#### Scenario: A failure while one is open
- **WHEN** the weekly run fails again and a `performance-regression` issue is still open
- **THEN** a comment with the new failures is added and no second issue is opened

#### Scenario: A pull request or a dispatch
- **WHEN** a check fails in a pull-request run or a manual dispatch
- **THEN** that run shows the failure and no issue is opened

### Requirement: A change to the request path or a shared resource carries a measurement
A change that adds work to the request path, or uses the serving runtime, the ledger pool, the writer's queue or the process's CPU allowance, SHALL include a performance task that records a before and after measurement or passes the isolation test, and `openspec/config.yaml` SHALL require this of every such change's tasks.

#### Scenario: A task is missing
- **WHEN** a change's design names the serving runtime or the ledger pool as used and its tasks hold no performance task
- **THEN** the change is not ready to apply
