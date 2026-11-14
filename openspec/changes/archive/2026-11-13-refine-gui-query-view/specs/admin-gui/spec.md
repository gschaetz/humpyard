# Spec Delta

## ADDED Requirements

### Requirement: Times can be shown in the viewer's time zone
The query bar SHALL offer `Local` (the time zone of the browser) and `UTC` for the times the chart shows, SHALL start at `Local`, and SHALL apply the choice to the axis ticks, the hover text of a point and the axis note, which SHALL name the zone in use. The choice SHALL change only how times are written; the data, the buckets and the requests to the gateway SHALL be the same in both.

#### Scenario: Local time
- **WHEN** the browser's zone is US Central and a point's bucket starts at 18:00 UTC
- **THEN** its hover text and axis tick show 12:00 (or 13:00 in daylight saving time) and the axis note names that zone

#### Scenario: UTC
- **WHEN** the person chooses `UTC`
- **THEN** the same point is written as 18:00 and the axis note says UTC, and the same data is shown

#### Scenario: A zone with a half-hour offset
- **WHEN** the browser's zone is 5 hours 30 minutes ahead of UTC
- **THEN** the ticks and hover text are the true local times of the buckets, written to the minute

#### Scenario: Changing the zone does not read again
- **WHEN** the person switches between `Local` and `UTC`
- **THEN** the chart is redrawn from the data already read and no request is made to the gateway

### Requirement: The ranking is ordered by its metric
The ranked bar view SHALL order its bars by the chosen metric's value, highest first, with groups that have no value last and ties by group name. A bar for a rate or a latency whose group had fewer than 20 calls in the window SHALL show the number of calls (under the requests metric the value is that number). The table's order SHALL NOT change.

#### Scenario: Requests
- **WHEN** the metric is requests
- **THEN** the group with the most requests is the first bar and each following bar is no longer than the one above it

#### Scenario: Latency
- **WHEN** the metric is p90 latency
- **THEN** the slowest group is the first bar, whatever the health of any group

#### Scenario: A rate from few calls
- **WHEN** a group has a 100% failure rate from one call
- **THEN** its bar is placed by that value and is labelled with the one call it stands on

#### Scenario: Requests need no count beside them
- **WHEN** the metric is requests and a group has one request
- **THEN** its bar shows the value 1 and no separate call count

#### Scenario: No value
- **WHEN** a group has no value for the metric
- **THEN** it is listed after every group that has one and says it has no value, not zero

#### Scenario: The table does not follow
- **WHEN** the ranking is shown
- **THEN** the table below it keeps its problems-first order and column sorting

## MODIFIED Requirements

### Requirement: The query is kept in the URL fragment and is treated as untrusted
The GUI SHALL keep the chosen query, time zone choice and refresh period in the URL fragment so a view can be bookmarked and shared, SHALL contain no credential or token in it, and SHALL accept only values from its fixed lists for the window, grouping, metric, time zone choice and refresh period, treating any other value as the default, and showing a filter value only as text.

#### Scenario: A shared link
- **WHEN** a link with a 7-day window, endpoint grouping and a target filter is opened
- **THEN** the query bar and the data show that query once the person has signed in

#### Scenario: A hostile fragment
- **WHEN** the fragment holds `window=<script>alert(1)</script>&metric=../x&refresh=1ms&zone=../x`
- **THEN** the view uses the defaults for all four and creates no element from the fragment

#### Scenario: A shared time zone choice
- **WHEN** a link with `zone=utc` is opened in a browser whose zone is not UTC
- **THEN** the times are shown in UTC
