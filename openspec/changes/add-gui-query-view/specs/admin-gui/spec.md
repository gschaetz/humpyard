## RENAMED Requirements

- FROM: `### Requirement: The models view shows how each target is doing`
- TO: `### Requirement: The query view shows how targets are doing`

## MODIFIED Requirements

### Requirement: The query view shows how targets are doing
The GUI SHALL provide a query view made of a query bar, a chart of the chosen metric and a table of targets, using only `/admin/v1/stats`, `/admin/v1/stats/series` and `/admin/v1/health`, and the table SHALL still be able to show, for the chosen window, each target's requests, answered calls, failure and fallback rates, response-signal rates over observed calls, tokens, cost, latency percentiles and endpoint health.

#### Scenario: A target with unobserved calls
- **WHEN** a target has no calls whose response signals are known in the window
- **THEN** the view shows no signal rate for it and does not show 0%

#### Scenario: The gateway is unreachable
- **WHEN** a request to the admin API fails
- **THEN** the view shows the error and does not keep showing the last numbers as current

#### Scenario: A gateway without the series endpoint
- **WHEN** the gateway answers 404 for `/admin/v1/stats/series` because it is older than the GUI
- **THEN** the table still shows, the chart says the gateway cannot draw a series, and the version warning is shown as before

## ADDED Requirements

### Requirement: The query bar chooses what is shown
The query bar SHALL offer a window, a grouping (route, target or endpoint), a metric (failure rate, fallback rate, p90 latency or requests) and optional route and target filters, SHALL query again when any of them changes, and SHALL start from a 24-hour window grouped by target on failure rate.

#### Scenario: Changing the window
- **WHEN** the window changes from 24 hours to 7 days
- **THEN** the chart and the table are both read again for 7 days, and the interval is chosen so the chart has a bounded number of buckets

#### Scenario: A filter that matches nothing
- **WHEN** a target filter matches no target in the window
- **THEN** the view says nothing matched and shows no chart and no rows, not an error

### Requirement: The chart shows the metric over time and as a ranking
The chart SHALL draw one line per top group for the chosen metric across the window's buckets, SHALL offer the same metric as a ranked bar view, SHALL leave a gap, not a zero, where a bucket has no value, SHALL tell the person when the series is truncated, SHALL identify each series by more than colour, and SHALL filter the table to a series when that series is chosen.

#### Scenario: A bucket with no calls
- **WHEN** a group has no calls in a bucket
- **THEN** the line has a gap there and the hover text for that point says there were no calls

#### Scenario: Choosing a series
- **WHEN** the person chooses one series in the chart or its legend
- **THEN** the table shows only that group, and choosing it again shows all groups

#### Scenario: A truncated series
- **WHEN** the answer says the row cap cut the oldest requests off
- **THEN** the chart marks the time before the earliest covered point as not shown, instead of drawing it as quiet

### Requirement: The table leads with four figures and puts problems first
Each row of the table SHALL show the group's endpoint health (for target and endpoint groupings, which have it), its requests with failure rate, its fallback rate and its latency, SHALL keep tokens, cost, answered calls and response-signal rates in an expandable detail row, and SHALL by default order groups whose endpoints are all cooling down first, then those with any endpoint not healthy, then by failure rate, then by fallback rate, then by requests, with groups that had no calls last.

#### Scenario: A failing target outranks a busy one
- **WHEN** one target has a 30% failure rate on 10 requests and another has 1% on 10,000
- **THEN** the first is listed above the second

#### Scenario: An unrated target
- **WHEN** a target has endpoints but no calls in the window
- **THEN** it is listed after every target that had calls, with no rate shown

#### Scenario: Grouping by route
- **WHEN** the grouping is route
- **THEN** the table shows no health column, because a route has no endpoint state of its own, and orders by failure rate

#### Scenario: Sorting by a column
- **WHEN** the person sorts by another column
- **THEN** the rows follow that column, and the default order can be restored

### Requirement: The query is kept in the URL fragment and is treated as untrusted
The GUI SHALL keep the chosen query and refresh period in the URL fragment so a view can be bookmarked and shared, SHALL contain no credential or token in it, and SHALL accept only values from its fixed lists for the window, grouping, metric and refresh period, treating any other value as the default, and showing a filter value only as text.

#### Scenario: A shared link
- **WHEN** a link with a 7-day window, endpoint grouping and a target filter is opened
- **THEN** the query bar and the data show that query once the person has signed in

#### Scenario: A hostile fragment
- **WHEN** the fragment holds `window=<script>alert(1)</script>&metric=../x&refresh=1ms`
- **THEN** the view uses the defaults for all three and creates no element from the fragment

### Requirement: The view refreshes itself without hammering the gateway
The query view SHALL refresh its chart and table on a period chosen in the query bar (off, 30 seconds, 1 minute or 5 minutes, starting at 1 minute), SHALL refresh only while the page is visible, SHALL NOT start a refresh while the previous one is still running, SHALL show when the data was last updated, SHALL wait longer between attempts after a failure, and SHALL stop and return to sign-in when the gateway refuses the credential.

#### Scenario: A hidden tab
- **WHEN** the page is hidden for ten minutes with a 1-minute period
- **THEN** no request is made while it is hidden, and one refresh runs when it becomes visible again if the data is older than the period

#### Scenario: A slow gateway
- **WHEN** a refresh has not answered when the next period arrives
- **THEN** no second request is started, and the next one is scheduled after the first ends

#### Scenario: The gateway fails
- **WHEN** a refresh fails
- **THEN** the view shows the error, marks the numbers on screen as out of date with the time they are from, and retries after longer waits that never exceed five minutes, going back to the chosen period after a success

#### Scenario: The gateway is busy
- **WHEN** a refresh is answered 503 `admin_busy` with a `Retry-After`
- **THEN** the view says the gateway is busy, keeps the earlier numbers marked with their time, and makes its next attempt no sooner than the `Retry-After`

#### Scenario: The credential is refused
- **WHEN** a refresh is answered 401 or 403
- **THEN** refreshing stops and the sign-in is shown, instead of repeating a request that will be refused

#### Scenario: Changing the query
- **WHEN** the window, grouping, metric, a filter or the refresh period changes while a refresh is running
- **THEN** the running request's answer is discarded, the new query is read at once, and the period starts again from then

#### Scenario: The period is off
- **WHEN** the refresh period is off
- **THEN** nothing is read until the query changes or the person asks, and the view shows the time its data is from

### Requirement: The chart is drawn within the strict policy
The chart SHALL be drawn by the GUI's own code as SVG whose appearance comes from the stylesheet, SHALL NOT use inline `style` attributes, inline scripts or a library that injects style or script elements, and SHALL leave the Content-Security-Policy unchanged.

#### Scenario: The policy still holds
- **WHEN** the built page is checked
- **THEN** it has no inline script or style, and the response headers are exactly those of every other response
