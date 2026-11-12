## ADDED Requirements

### Requirement: Heavy reads run off the serving runtime on their own connections
Statistics and session reads SHALL do their per-row work on a thread outside the serving runtime's workers and SHALL read through a read-only connection pool separate from the writer's, so that a read neither stops the runtime from serving nor takes a connection the writer needs.

#### Scenario: The runtime keeps serving
- **WHEN** a statistics read folds 50,000 decision rows while requests are served by a runtime with one worker
- **THEN** request latency stays within the isolation budget

#### Scenario: Reads cannot starve the writer
- **WHEN** several reads are attempted while entries are queued
- **THEN** the writer still finds a connection and no entry is dropped

### Requirement: Reads cannot cause entries to be lost
A read of the ledger SHALL NOT cause the writer's queue to fill and drop an entry, and the number of dropped entries SHALL be observable after a read-heavy period.

#### Scenario: Dropped entries are counted
- **WHEN** the writer's queue is full and an entry is recorded
- **THEN** the entry is dropped, the counter rises, and a read-heavy run in which the counter rose is reported as a failure by the isolation test

### Requirement: Pruning holds the write lock only briefly
The ledger SHALL delete expired decisions, override history and quota samples in bounded batches, each its own statement, so that no single write holds the database's write lock for a time near the writer's busy timeout, and SHALL leave the rows it has not yet reached for the next prune.

#### Scenario: A large prune
- **WHEN** 500,000 decision rows are older than the retention and are pruned
- **THEN** they are deleted in batches of at most 10,000, and an override written meanwhile completes within one batch's duration plus its own

#### Scenario: A prune interrupted by shutdown
- **WHEN** the gateway stops part-way through a prune
- **THEN** the rows already deleted stay deleted, no row newer than the cutoff is deleted, and the next prune deletes the rest

### Requirement: A read that cannot start fails quickly
A read of the ledger SHALL wait at most two seconds for a connection and one second for a lock, and a read that exceeds either SHALL fail so the caller can answer 503 busy, rather than waiting for the library defaults of thirty seconds and five.

#### Scenario: The read pool is exhausted
- **WHEN** both read connections are held and another read is made
- **THEN** it fails within about two seconds and the writer is unaffected
