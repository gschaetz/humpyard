## ADDED Requirements

### Requirement: humpyard ui serves the GUI
The CLI SHALL provide `humpyard ui --url URL [--listen ADDR]`, which serves the admin GUI on `ADDR` (default `127.0.0.1:8787`) with the same security headers as the GUI's own responses. The command SHALL NOT be reachable from `serve`.

#### Scenario: Default
- **WHEN** `humpyard ui --url https://gateway.example` is run
- **THEN** the GUI is served on `127.0.0.1:8787`

### Requirement: humpyard ui forwards only the admin API
`humpyard ui` SHALL forward requests under `/admin/v1` to the gateway at `URL`. It SHALL normalise a request path before matching it and forward only a path that is still under `/admin/v1` afterwards, SHALL forward nothing else, SHALL NOT follow redirects, and SHALL bound each forwarded request in time and size.

#### Scenario: The admin API is forwarded
- **WHEN** a request is made for `/admin/v1/health`
- **THEN** it is forwarded to the gateway and the gateway's answer is returned

#### Scenario: Only the admin API is forwarded
- **WHEN** a request is made to the `humpyard ui` address for `/v1/chat/completions`
- **THEN** it is answered with 404 and nothing is forwarded

#### Scenario: A path that leaves the prefix
- **WHEN** a request is made for `/admin/v1/../v1/chat/completions`, in plain or percent-encoded form
- **THEN** it is answered with 404 and nothing is forwarded

#### Scenario: Redirects are not followed
- **WHEN** the gateway answers a forwarded request with a redirect
- **THEN** the redirect is returned to the browser and not followed by the command

### Requirement: humpyard ui can keep the admin credential out of the browser
With `--credential-env NAME`, `humpyard ui` SHALL read the admin credential from that environment variable and add it to every forwarded request in place of any credential the browser sent, so the credential never reaches the browser. Because anyone who can reach its port then acts with that credential, it SHALL refuse a `--listen` address that is not a loopback address unless `--allow-remote-with-credential` is also given. Without `--credential-env` the browser supplies its own credential.

#### Scenario: A credential and a public address
- **WHEN** `--credential-env` is given with `--listen 0.0.0.0:8787` and no `--allow-remote-with-credential`
- **THEN** the command exits with an error that explains why

#### Scenario: The credential stays out of the browser
- **WHEN** `--credential-env` is used
- **THEN** no response from the command contains the credential, and forwarded requests carry it instead of any the browser sent

### Requirement: humpyard ui refuses cross-site and rebound requests
`humpyard ui` SHALL answer a request only when its `Host` header names a loopback host (`localhost`, `127.0.0.1`, `[::1]`) or a host given with `--allow-host NAME` (repeatable), and SHALL refuse to start on a non-loopback `--listen` address unless at least one `--allow-host` is given. It SHALL answer 403 to a request that carries `Sec-Fetch-Site: cross-site`, and to a request other than GET or HEAD whose `Origin` header is present and is not its own origin.

#### Scenario: A rebound host name
- **WHEN** a request arrives with `Host: attacker.example:8787`
- **THEN** the answer is 403 and nothing is forwarded

#### Scenario: A cross-site request
- **WHEN** a request arrives marked `Sec-Fetch-Site: cross-site`
- **THEN** the answer is 403

#### Scenario: A state-changing request from another origin
- **WHEN** a POST arrives with an `Origin` that is not the command's own
- **THEN** the answer is 403 and nothing is forwarded

#### Scenario: A named host
- **WHEN** the command is started with `--listen 0.0.0.0:8787 --allow-host ui.internal:8787`
- **THEN** requests with `Host: ui.internal:8787` are answered and requests with any other host are refused

#### Scenario: A public address with no allowed host
- **WHEN** `--listen 0.0.0.0:8787` is given with no `--allow-host`
- **THEN** the command exits with an error that explains why
