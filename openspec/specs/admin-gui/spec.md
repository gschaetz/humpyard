# admin-gui Specification

## Purpose
TBD - created by archiving change add-admin-gui-delivery. Update Purpose after archive.

## Requirements

### Requirement: The gateway serves no GUI
The gateway SHALL NOT serve the admin GUI under any configuration: `/admin/ui/` and any other GUI path SHALL be answered as an unknown path is. The GUI is served only by the `humpyard ui` command.

#### Scenario: A gateway with the admin API on
- **WHEN** a gateway with `[admin]` credentials configured is asked for `/admin/ui/`
- **THEN** the answer is 404, as for any unknown path

### Requirement: Only embedded files are served
`humpyard ui` SHALL serve the GUI from files embedded in the binary, looked up by exact path, and SHALL NOT read the filesystem to answer a request. A path with no file extension that matches no file SHALL be answered with the GUI's `index.html` so client-side routes load; any other unknown path SHALL be answered with 404.

#### Scenario: A traversal attempt
- **WHEN** a request is made for `/../../etc/passwd` in any encoding
- **THEN** the answer is 404 and no file outside the embedded set is read

#### Scenario: A client-side route
- **WHEN** a request is made for `/models` and no such file exists
- **THEN** the answer is the GUI's `index.html` with status 200

#### Scenario: A missing asset
- **WHEN** a request is made for `/assets/nope.js`
- **THEN** the answer is 404

### Requirement: Responses carry restrictive headers
Every response SHALL carry a Content-Security-Policy that allows scripts, styles and connections only from the page's own origin and forbids framing (`default-src 'self'`, `script-src 'self'`, `style-src 'self'`, `connect-src 'self'`, `frame-ancestors 'none'`, `base-uri 'none'`), `X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`. `index.html` SHALL be sent with `Cache-Control: no-cache`; files whose names carry a content hash SHALL be sent as immutable.

#### Scenario: The policy on the page
- **WHEN** the GUI's page is requested
- **THEN** the response has a Content-Security-Policy with no `unsafe-inline` and no `unsafe-eval`

#### Scenario: Caching
- **WHEN** a hashed asset and `index.html` are requested
- **THEN** the asset is cacheable as immutable and `index.html` is `no-cache`

### Requirement: The files grant nothing and every data call is authorised by the admin API
The GUI's files SHALL need no credential and SHALL contain no data from the gateway. Every call to `/admin/v1` SHALL carry an admin credential (the user's, or the one `humpyard ui` was given) and be authorised by the admin API exactly as for any other client.

#### Scenario: The page without a credential
- **WHEN** the GUI's page is requested with no credential
- **THEN** the page is served

#### Scenario: Data without a credential
- **WHEN** `/admin/v1/config` is requested through `humpyard ui` started without `--credential-env`, with no credential
- **THEN** the answer is 401 as from the gateway

### Requirement: The GUI talks only to the admin API of its own origin and keeps its credential for the session
The GUI SHALL send requests only to `/admin/v1` on the origin it was loaded from, and, when `humpyard ui` holds the admin credential and asks for its access token, one request to `/ui/session` on that origin with the token in the body (never in a URL). When the user supplies a credential, the GUI SHALL keep it for the browser session only (not in `localStorage`, a cookie or any storage that outlives the session), and SHALL hold no configuration or data of its own beyond what it displays.

#### Scenario: The access token
- **WHEN** the service answers a data request with `ui_token_required`
- **THEN** the GUI asks for the token, sends it once in a request body to `/ui/session`, does not store it, and asks the data request again

#### Scenario: Credential lifetime
- **WHEN** the browser session ends
- **THEN** nothing the GUI stored lets a new session act as the user

### Requirement: Every string from the API is rendered as text
The GUI SHALL render every string it receives from the API, including model names, session ids, key ids and provider error text, as text and never as markup, and its source SHALL NOT use APIs that insert raw HTML.

#### Scenario: A hostile model name
- **WHEN** a decision's requested model is `<img src=x onerror=alert(1)>`
- **THEN** the GUI shows that exact text and creates no element from it

### Requirement: The GUI checks the contract version of the gateway it talks to
On load the GUI SHALL read the admin API's contract version from `/admin/v1/openapi.json` and, when it is not one the GUI was built for, SHALL say so before showing any data, naming both versions, and SHALL still show the data.

#### Scenario: A newer gateway
- **WHEN** the gateway's contract version is not in the GUI's supported set
- **THEN** the GUI shows a notice naming the gateway's version and its own, and does not hide the data

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
Each row of the table SHALL show the group's endpoint health (for target and endpoint groupings, which have it), its requests with failure rate, its fallback rate and its latency, SHALL keep tokens, cost, answered calls and response-signal rates in an expandable detail row, and SHALL by default order groups whose endpoints are all cooling down first, then those with any endpoint not healthy, then by failure rate, then by fallback rate, then by requests, with groups that had no calls last, ranking each rate by the lower end of its 95% confidence interval over the attempts or requests behind it, so a rate on few calls ranks below the same rate on many, and showing the raw rate.

#### Scenario: A failing target outranks a busy one
- **WHEN** one target has a 30% failure rate on 10 requests and another has 1% on 10,000
- **THEN** the first is listed above the second

#### Scenario: A lone failure does not outrank a well-sampled one
- **WHEN** one target has failed its only attempt (100%) and another has failed 30 of 100 attempts (30%)
- **THEN** the second is listed above the first, and both rows show their raw failure rate

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
