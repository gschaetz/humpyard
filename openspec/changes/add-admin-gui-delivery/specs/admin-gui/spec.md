## ADDED Requirements

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
The GUI SHALL send requests only to `/admin/v1` on the origin it was loaded from. When the user supplies a credential, the GUI SHALL keep it for the browser session only (not in `localStorage`, a cookie or any storage that outlives the session), and SHALL hold no configuration or data of its own beyond what it displays.

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

### Requirement: The models view shows how each target is doing
The GUI SHALL provide a models view that lists each target with, for the chosen window, its requests, answered calls, failure rate, fallback rate, the rate of each response signal over the calls that were observed, tokens, cost, latency percentiles and endpoint health, using only `/admin/v1/stats` and `/admin/v1/health`.

#### Scenario: A target with unobserved calls
- **WHEN** a target has no calls whose response signals are known in the window
- **THEN** the view shows no signal rate for it and does not show 0%

#### Scenario: The gateway is unreachable
- **WHEN** a request to the admin API fails
- **THEN** the view shows the error and does not keep showing the last numbers as current
