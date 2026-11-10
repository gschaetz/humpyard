## ADDED Requirements

### Requirement: Statistics can be bucketed over time
A statistics series SHALL report, for each group over a window, a sequence of equal time buckets, each with the group's requests, attempts, failures, failure rate, fallbacks, fallback rate and attempt latency p50, p90 and p99, counted by the same rules as the unbucketed statistics and with percentiles by the nearest-rank method over the bucket's attempts.

#### Scenario: Buckets add up
- **WHEN** a series and the unbucketed statistics are read for the same window, group and filters
- **THEN** each group's bucket requests, attempts and failures sum to the group's totals

#### Scenario: A quiet bucket
- **WHEN** no request of a group falls in a bucket
- **THEN** the bucket is still present with zero counts, and its rates and percentiles are null rather than 0

#### Scenario: Bucket edges
- **WHEN** a series is read with an interval of 5 minutes
- **THEN** every bucket starts at a whole multiple of 5 minutes, so the same instant falls in the same bucket on every read

### Requirement: A series is bounded and says when it is partial
A statistics series SHALL reject a window longer than the decision retention, an interval that would give more than 400 buckets or less than one minute, and a group limit above 25, SHALL keep only the groups with the most requests, and SHALL read at most the same number of decision rows as the unbucketed statistics, reporting `truncated` and the time its oldest counted row covers when the cap cut the window short.

#### Scenario: Too many buckets
- **WHEN** a series is asked for a 30-day window with a 1-minute interval
- **THEN** the answer is a 400 naming the interval limit and nothing is computed

#### Scenario: Only the busiest groups
- **WHEN** a window has 40 targets and the limit is 10
- **THEN** the series holds the 10 targets with the most requests, in that order, and says how many groups it left out

#### Scenario: The row cap cuts the window
- **WHEN** a window holds more decision rows than the cap
- **THEN** the series is marked truncated and states the earliest time it covers, so buckets before it are not mistaken for quiet ones

### Requirement: A series follows the statistics read model and is read-only
A statistics series SHALL honour the same per-key visibility, filters and group choices as the unbucketed statistics (route, target or endpoint, and the `key` filter on the admin API), SHALL NOT change routing or write anything, and SHALL return empty groups rather than an error for an empty window.

#### Scenario: Filters narrow a series
- **WHEN** a series is read with a `target` filter
- **THEN** only attempts on that target are counted in every bucket

#### Scenario: Empty window
- **WHEN** no request falls in the window
- **THEN** the answer has no groups and the window and bucket edges it covers
