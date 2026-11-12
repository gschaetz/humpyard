# usage-ledger Specification

## Purpose
Defines how token usage and cost are measured and persisted for every upstream call.

## Requirements

### Requirement: Every upstream call is recorded
The gateway SHALL record one ledger entry for each upstream call, including routing-time judge calls, with timestamp, key id (or none), session id, route, target, provider, model, call kind (answer or judge), token counts (input, cached input, output, reasoning), cost in USD and outcome.

#### Scenario: Answer call
- **WHEN** a request is served by target `fast`
- **THEN** the ledger gains an entry of kind answer naming the key, route, target, provider, model, tokens and cost

#### Scenario: Judge call
- **WHEN** an `llm_classifier` route calls its judge before answering
- **THEN** the ledger gains a separate entry of kind judge charged to the same key

### Requirement: Streaming usage recorded at stream end
For streamed responses the gateway SHALL record usage when the stream ends. If the stream ends without usage, the gateway SHALL record the entry with the missing-usage marker and, when text was generated and delivered, an output token estimate (and its cost) derived from the delivered bytes; input tokens SHALL stay zero.

#### Scenario: Normal stream
- **WHEN** a streamed answer completes with a usage chunk
- **THEN** its entry carries those token counts

#### Scenario: Stream without usage
- **WHEN** the provider reports no usage for a stream that delivered text
- **THEN** the entry is recorded with the missing-usage marker, an estimated positive output count and its cost, and a warning is logged

#### Scenario: Cut stream
- **WHEN** a stream is cut by the grace period or the client disconnects after text was delivered
- **THEN** the entry is `cancelled`, marked as an estimate, with the estimated output tokens

#### Scenario: Nothing generated
- **WHEN** a stream ends without usage and without any generated text
- **THEN** the entry has zero tokens and zero cost

### Requirement: Cost from endpoint prices
The gateway SHALL compute cost from the endpoint's configured USD-per-million-token prices (input, output, optional cached input) and the reported tokens.

#### Scenario: Priced call
- **WHEN** an endpoint priced at $1 input and $4 output per million serves 1,000 input and 500 output tokens
- **THEN** the entry's cost is $0.003

### Requirement: Prices required for budgeted keys
When any key has a USD budget, startup SHALL fail unless every endpoint declares a price (zero is allowed), so that no endpoint is silently free.

#### Scenario: Missing price
- **WHEN** a key has a daily USD limit and an endpoint has no price
- **THEN** startup fails naming the endpoint

### Requirement: Recording never blocks requests
Ledger writes SHALL happen asynchronously; a failed or backlogged write SHALL be logged and counted and SHALL NOT fail or delay the client's response.

#### Scenario: Database unavailable
- **WHEN** the ledger file cannot be written
- **THEN** requests continue to be served and the failures are logged

### Requirement: Recovery after restart
On startup the gateway SHALL rebuild each key's spend for the current periods from the ledger.

#### Scenario: Restart mid-day
- **WHEN** a key spent $2 today and the gateway restarts
- **THEN** its daily spend is $2 after startup

### Requirement: Cancelled requests still record their spend
Usage already incurred by a request SHALL be recorded even if the request is cancelled before a response is returned, whether by the client disconnecting or by shutdown.

#### Scenario: Client disconnects during a classifier route
- **WHEN** a client disconnects after a route's judge call has returned but before the answer completes
- **THEN** the judge call's usage is recorded and charged to the key

### Requirement: Entries survive a graceful exit
The gateway SHALL write all queued entries before exiting after a termination signal.

#### Scenario: Stop right after a request
- **WHEN** a request finishes and the gateway receives SIGTERM before the ledger writer has run
- **THEN** the entry is in the ledger when the process has exited

### Requirement: Reasoning tokens are part of output
The gateway SHALL treat reasoning tokens reported by a provider as part of that call's output tokens, record them for information, and SHALL NOT add them again to cost or to token budgets.

#### Scenario: Reasoning model call
- **WHEN** a provider reports 100 completion tokens of which 90 are reasoning tokens, and 10 prompt tokens
- **THEN** the call is costed on 10 input and 100 output tokens, counts 110 tokens against token budgets, and the ledger entry records 90 reasoning tokens as detail

#### Scenario: Totals after a restart
- **WHEN** spend is rebuilt from the ledger after a restart
- **THEN** reasoning tokens are not added to the rebuilt token totals

### Requirement: Session queries are indexed
The ledger SHALL keep an index over usage rows by session id and time, added by an in-place, idempotent schema upgrade that preserves every existing row.

#### Scenario: Upgrading an older database
- **WHEN** a database at the previous schema version is opened
- **THEN** the index exists afterwards and all earlier rows are intact

### Requirement: A call's entry carries the response signals
Each ledger entry for an answer call SHALL carry the four response signals (`empty_output`, `truncated_output`, `bad_tool_call`, `cut_stream`) for the call it describes, attributed to that entry's target, provider and model. A signal SHALL read as unknown, not as unset, on an entry written before the signals existed, on a judge call, and on a call whose response was not observed. Entries SHALL keep the signals for as long as they keep the entry.

#### Scenario: A flagged call
- **WHEN** a request is served by target `a` and its answer ended on the length limit
- **THEN** the entry for that call names target `a` and carries `truncated_output`

#### Scenario: A clean call
- **WHEN** a request is served and its answer is whole
- **THEN** the entry carries all four signals as unset, not as unknown

#### Scenario: Older rows
- **WHEN** an entry written before this requirement is read
- **THEN** its signals read as unknown and nothing fails

#### Scenario: A streamed answer is flagged when it ends
- **WHEN** a streamed answer ends after its decision was recorded
- **THEN** the entry written at the end of the stream carries its signals

#### Scenario: A judge call
- **WHEN** a routing-time judge call is recorded
- **THEN** its signals read as unknown

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
