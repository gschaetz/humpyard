## ADDED Requirements

### Requirement: Client statistics reads are limited and share results
The gateway SHALL run at most one heavy client statistics read at a time (statistics and session views on `/v1`), separate from the administrative gate, SHALL answer another that arrives meanwhile with 503 `stats_busy` and a `Retry-After` header without queueing it, and SHALL answer an identical read by the same key within ten seconds from the earlier result.

#### Scenario: Busy
- **WHEN** one client statistics read is running and another is made
- **THEN** the second is answered 503 `stats_busy` with `Retry-After`

#### Scenario: A key never sees another key's result
- **WHEN** two keys make the same statistics read within ten seconds
- **THEN** each answer is computed over that key's own requests

#### Scenario: A client cannot hold the administrative gate
- **WHEN** client statistics reads run continuously
- **THEN** an administrative statistics read is still answered, not refused as busy

### Requirement: Statistics may be up to ten seconds old
A statistics answer SHALL reflect the ledger no more than ten seconds before it was asked for, and SHALL carry the time it was computed.

#### Scenario: Freshness is visible
- **WHEN** a statistics answer is read
- **THEN** it states when it was computed, and a repeat within ten seconds states the same time
