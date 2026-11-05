## ADDED Requirements

### Requirement: The GUI is off unless enabled
The gateway SHALL serve no GUI path unless `[admin.ui] enabled` is true. With it off or absent, every path under `/admin/ui/` SHALL answer as any unknown path does.

#### Scenario: Off by default
- **WHEN** a configuration has an `[admin]` section and no `[admin.ui]`
- **THEN** a request for `/admin/ui/` is answered with 404, as an unknown path is

#### Scenario: Enabled
- **WHEN** `[admin.ui] enabled = true` and the admin API is configured
- **THEN** a request for `/admin/ui/` is answered with the GUI's page

### Requirement: The GUI is served where the admin API is
The GUI SHALL be served at `/admin/ui/` on the listener that serves `/admin/v1`: the main listener, or the address in `[admin] listen` when one is set. Enabling it without any admin credential configured SHALL be a configuration error.

#### Scenario: On the admin listener
- **WHEN** `[admin] listen` names its own address and the GUI is enabled
- **THEN** `/admin/ui/` is served on that address and not on the main listener

#### Scenario: No admin credential
- **WHEN** `[admin.ui] enabled = true` and `[admin]` defines no credential
- **THEN** the configuration is rejected naming `admin.ui`

### Requirement: Only embedded files are served
The gateway SHALL serve the GUI from files embedded in the binary, looked up by exact path, and SHALL NOT read the filesystem to answer a request under `/admin/ui/`. A path with no file extension that matches no file SHALL be answered with the GUI's `index.html` so client-side routes load; any other unknown path SHALL be answered with 404.

#### Scenario: A traversal attempt
- **WHEN** a request is made for `/admin/ui/../../etc/passwd` in any encoding
- **THEN** the answer is 404 and no file outside the embedded set is read

#### Scenario: A client-side route
- **WHEN** a request is made for `/admin/ui/models` and no such file exists
- **THEN** the answer is the GUI's `index.html` with status 200

#### Scenario: A missing asset
- **WHEN** a request is made for `/admin/ui/assets/nope.js`
- **THEN** the answer is 404

### Requirement: Responses carry restrictive headers
Every GUI response SHALL carry a Content-Security-Policy that allows scripts, styles and connections only from the page's own origin and forbids framing (`default-src 'self'`, `script-src 'self'`, `style-src 'self'`, `connect-src 'self'`, `frame-ancestors 'none'`, `base-uri 'none'`), `X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`. `index.html` SHALL be sent with `Cache-Control: no-cache`; files whose names carry a content hash SHALL be sent as immutable.

#### Scenario: The policy on the page
- **WHEN** `/admin/ui/` is requested
- **THEN** the response has a Content-Security-Policy with no `unsafe-inline` and no `unsafe-eval`

#### Scenario: Caching
- **WHEN** a hashed asset and `index.html` are requested
- **THEN** the asset is cacheable as immutable and `index.html` is `no-cache`

### Requirement: Serving the page grants nothing
Files under `/admin/ui/` SHALL need no credential and SHALL contain no data from the gateway. Every call the page makes to `/admin/v1` SHALL be authenticated and authorised by the admin API exactly as for any other client.

#### Scenario: The page without a credential
- **WHEN** `/admin/ui/` is requested with no credential
- **THEN** the page is served

#### Scenario: Data without a credential
- **WHEN** `/admin/v1/config` is requested with no credential
- **THEN** the answer is 401 as before

### Requirement: The GUI talks only to the admin API of its own origin and keeps no state
The GUI SHALL send requests only to `/admin/v1` on the origin it was loaded from, SHALL ask the user for an admin credential and keep it for the browser session only (not in `localStorage`, a cookie or any storage that outlives the session), and SHALL hold no configuration or data of its own beyond what it displays.

#### Scenario: Credential lifetime
- **WHEN** the browser session ends
- **THEN** nothing the GUI stored lets a new session act as the user

### Requirement: The GUI checks the contract version of the gateway it talks to
On load the GUI SHALL read the admin API's contract version from `/admin/v1/openapi.json` and, when it is not one the GUI was built for, SHALL say so before showing any data, naming both versions.

#### Scenario: A newer gateway
- **WHEN** the gateway's contract version is not in the GUI's supported set
- **THEN** the GUI shows a notice naming the gateway's version and its own, and does not hide the data
