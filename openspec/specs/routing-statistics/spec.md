# routing-statistics Specification

## Purpose
Defines read-only performance statistics over routing decisions and usage: per route, target and endpoint failure mix, fallback rate, latency percentiles and cost per request, readable per key and shown in explain.

## Requirements

### Requirement: Performance statistics per route, target and endpoint
The gateway SHALL answer `GET /v1/stats` with, for each group (route, target or endpoint, as `group_by` selects) over the requested window: the number of requests and attempts, the attempt outcome counts by class, the fallback rate, attempt latency percentiles (p50, p90, p99), and the average cost and tokens per request.

#### Scenario: Failure mix is visible
- **WHEN** a target returned rate limits for part of the window
- **THEN** its group shows the `rate_limit` count and its share of attempts

#### Scenario: Latency percentiles
- **WHEN** attempts took known times
- **THEN** the reported p50, p90 and p99 match those times by the nearest-rank method

### Requirement: Statistics follow the decision log's read model
A key SHALL see statistics over its own requests only, an open gateway over all, and the window SHALL be bounded by the decision retention.

#### Scenario: Isolation
- **WHEN** two keys have made requests
- **THEN** each key's statistics count only its own

### Requirement: Statistics are read-only
Computing or reading statistics SHALL NOT change routing, and an empty window SHALL return empty groups rather than an error.

#### Scenario: Empty window
- **WHEN** no requests fall inside the window
- **THEN** the response is 200 with no groups

### Requirement: Explain shows recent behavior
When a ledger is configured, `explain` SHALL report each eligible target's recent failure rate and p50 latency (over the default window) alongside its health.

#### Scenario: Annotated plan
- **WHEN** explain is asked about a route whose first target has been failing
- **THEN** the report shows that target's failure rate

### Requirement: Statistics report cache usage
Every group in the statistics response SHALL report its token totals (fresh input, cached input, cache creation and output) and a `cache_hit_ratio` equal to cached input tokens divided by the sum of fresh input, cached input and cache-creation tokens, or null when that sum is zero. Usage rows marked as missing usage SHALL NOT count toward the ratio.

#### Scenario: A cached prompt
- **WHEN** answered calls report cached input tokens
- **THEN** their group's `cache_hit_ratio` is the cached share of its prompt tokens

### Requirement: Statistics can be grouped by session
`GET /v1/stats?group_by=session` SHALL return one row per session id found on the caller's answered calls in the window, each with its call count, cost, token totals, `cache_hit_ratio`, the `provider/model` endpoints used in order of first use, its number of switches, and its first and last call times. Rows SHALL be limited by `limit` (default 20, at most 100) and ordered by `order_by` (`cost`, `requests`, `switches` or `recent`). Calls without a session id SHALL be counted, not grouped.

#### Scenario: Top sessions by cost
- **WHEN** a key asks for `group_by=session&order_by=cost&limit=5`
- **THEN** it receives at most its five costliest sessions, and the response states how many answered calls had no session id

#### Scenario: Another key's sessions
- **WHEN** a different key asks for the same view
- **THEN** it sees only its own sessions

### Requirement: Switches between endpoints are reported
`GET /v1/stats/switches` SHALL report, over a window, the number of sessions and how many switched endpoints; the number of switches, where a switch is two consecutive answered calls of one session on different `provider/model` endpoints; the warm tokens abandoned (the cached input plus cache-creation tokens of the call before each switch); the extra cost those tokens imply at the destination endpoint's configured price difference between fresh and cached input, with the number of switches that could be priced; that cost's share of total spend; the endpoint pairs that switch most; and the average calls and cost of sessions that stayed versus switched, with a note that the comparison is descriptive.

#### Scenario: A session that switched once
- **WHEN** a session's second call is on a different endpoint and the first call left 10,000 tokens cached
- **THEN** the report counts one switch and 10,000 warm tokens abandoned, and prices them if the destination endpoint has a price

#### Scenario: An unpriced destination
- **WHEN** the destination endpoint has no configured price
- **THEN** its tokens are counted but no cost is estimated for it, and the report says so

### Requirement: Session coverage is reported
The switch report SHALL state the share of answered calls in the window that carried a session id.

#### Scenario: Clients that send no session id
- **WHEN** most answered calls carry no session id
- **THEN** the coverage ratio is low and the report's session numbers are labelled as covering only those calls

### Requirement: Session views are bounded and read-only
Session queries SHALL scan at most a fixed number of usage rows (newest first), SHALL report when the window held more, and SHALL NOT change any state, read any content, or affect routing.

#### Scenario: A very large window
- **WHEN** the window holds more usage rows than the cap
- **THEN** the response is marked `truncated` and covers the newest rows

### Requirement: Statistics report the response signals per target
`GET /v1/stats` and its admin counterpart SHALL report, for each group over the requested window, how many answer calls were observed and how many carried each response signal, so a rate is the count of a signal over the observed calls. A call whose signals are unknown SHALL count in neither figure. The figures SHALL follow the same isolation and window rules as the other statistics.

#### Scenario: A rate can be computed
- **WHEN** a target served ten observed calls and two of them carried `truncated_output`
- **THEN** its group reports ten observed calls and two truncated

#### Scenario: Unknown calls are left out
- **WHEN** a window holds calls written before the signals existed
- **THEN** they do not count as observed and do not lower any rate

#### Scenario: Isolation
- **WHEN** two keys have made requests
- **THEN** each key's response-signal figures count only its own calls
