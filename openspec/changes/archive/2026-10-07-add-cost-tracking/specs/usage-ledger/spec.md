# Spec Delta

## Purpose

Defines how token usage and cost are measured and persisted for every upstream call.

## ADDED Requirements

### Requirement: Every upstream call is recorded
The gateway SHALL record one ledger entry for each upstream call, including routing-time judge calls, with timestamp, key id (or none), session id, route, target, provider, model, call kind (answer or judge), token counts (input, cached input, output, reasoning), cost in USD and outcome.

#### Scenario: Answer call
- **WHEN** a request is served by target `fast`
- **THEN** the ledger gains an entry of kind answer naming the key, route, target, provider, model, tokens and cost

#### Scenario: Judge call
- **WHEN** an `llm_classifier` route calls its judge before answering
- **THEN** the ledger gains a separate entry of kind judge charged to the same key

### Requirement: Streaming usage recorded at stream end
For streamed responses the gateway SHALL record usage when the stream ends, and if the stream ends without usage it SHALL record the entry with zero tokens and a missing-usage marker.

#### Scenario: Normal stream
- **WHEN** a streamed answer completes with a usage chunk
- **THEN** its entry carries those token counts

#### Scenario: Stream without usage
- **WHEN** the provider reports no usage for a stream
- **THEN** the entry is recorded with the missing-usage marker and a warning is logged

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
