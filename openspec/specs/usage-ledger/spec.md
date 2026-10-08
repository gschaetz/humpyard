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
