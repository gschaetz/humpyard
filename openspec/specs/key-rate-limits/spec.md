# key-rate-limits Specification

## Purpose
TBD - created by archiving change add-key-rate-limits. Update Purpose after archive.

## Requirements

### Requirement: A key can be limited to a number of requests per minute
A key with `max_requests_per_minute = N` SHALL be admitted for at most N requests in any 60-second window. A request that would make it N+1 SHALL be refused with status 429 and a `Retry-After` header giving the whole seconds until the oldest admitted request leaves the window, and SHALL NOT reach an upstream provider. A key without the setting SHALL have no limit.

#### Scenario: The limit is reached
- **WHEN** a key with a limit of 3 makes a fourth request within 60 seconds of its first
- **THEN** the fourth is answered 429 with `Retry-After`, and no upstream call is made for it

#### Scenario: The window slides
- **WHEN** 60 seconds have passed since the oldest of the key's 3 admitted requests
- **THEN** the next request is admitted

#### Scenario: No limit set
- **WHEN** a key has no `max_requests_per_minute`
- **THEN** no request of that key is refused for its rate

### Requirement: A key can be limited to a number of concurrent requests
A key with `max_concurrent = N` SHALL have at most N requests in flight. A request is in flight from admission until its response has ended: for a buffered answer when it is sent, for a streamed answer when the stream ends, fails or the client disconnects. A request that would make it N+1 SHALL be refused with status 429 and `Retry-After: 1`, and SHALL NOT reach an upstream provider.

#### Scenario: A stream holds its permit
- **WHEN** a key with `max_concurrent = 1` has a stream open and makes a second request
- **THEN** the second is answered 429 and the stream is unaffected

#### Scenario: The permit is released at the end of the stream
- **WHEN** the open stream finishes
- **THEN** the next request is admitted

#### Scenario: A client that disconnects releases its permit
- **WHEN** the client of an open stream goes away
- **THEN** the key's permit is released and the next request is admitted

#### Scenario: A failed request releases its permit
- **WHEN** a request fails after admission
- **THEN** its permit is released

### Requirement: Limits apply per authenticated key, after authentication and authorisation, before the budget
Limits SHALL be checked only for requests that carry a valid key and a route that key may use, and before the key's budget is checked. Each key's counters SHALL be independent of every other key's, and a request that fails authentication SHALL NOT change any key's counters. Only requests that can reach a provider SHALL be counted and limited; reading statistics, decisions or configuration SHALL NOT.

#### Scenario: An unauthenticated caller
- **WHEN** a caller with an invalid key sends many requests
- **THEN** no key's allowance is consumed

#### Scenario: Another key
- **WHEN** one key is at its limit
- **THEN** a different key's requests are admitted as usual

#### Scenario: Reads are not limited
- **WHEN** a key at its request limit asks for `/v1/stats`
- **THEN** the answer is the statistics, not a 429

### Requirement: A refusal for a limit is visible and well formed
A refusal SHALL carry an OpenAI-style error body whose `code` is `rate_limit_exceeded` and whose message names the limit, SHALL be recorded as a decision with the outcome `rate_limited`, and SHALL be counted in `humpyard_refusals_total` with the reason `rate_limited`. It SHALL NOT start a failover or touch any provider's health.

#### Scenario: The body and the record
- **WHEN** a request is refused for its rate
- **THEN** the body's `error.code` is `rate_limit_exceeded`, the decision's outcome is `rate_limited`, and `humpyard_refusals_total{reason="rate_limited"}` rises by one

### Requirement: Limits follow a configuration reload
A reload SHALL apply a changed or removed limit from then on. A key no longer in the configuration SHALL have its counters dropped. Requests already in flight SHALL keep their permits until they end.

#### Scenario: A limit is raised by a reload
- **WHEN** a key at its limit of 3 has the limit raised to 5 by a reload
- **THEN** its next request is admitted
