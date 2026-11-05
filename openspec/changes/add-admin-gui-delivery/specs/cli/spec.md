## ADDED Requirements

### Requirement: humpyard ui serves the GUI and forwards the admin API
The CLI SHALL provide `humpyard ui --url URL [--listen ADDR]`, which serves the admin GUI on `ADDR` (default `127.0.0.1:8787`) and forwards requests under `/admin/v1` to the gateway at `URL`. It SHALL forward nothing else, SHALL NOT follow redirects, SHALL bound each forwarded request in time and size, and SHALL send the same security headers as the built-in GUI. The command SHALL NOT be reachable from `serve`.

#### Scenario: Default
- **WHEN** `humpyard ui --url https://gateway.example` is run
- **THEN** the GUI is served on `127.0.0.1:8787` and `/admin/v1/*` requests are forwarded to the gateway

#### Scenario: Only the admin API is forwarded
- **WHEN** a request is made to the `humpyard ui` address for `/v1/chat/completions`
- **THEN** it is answered with 404 and nothing is forwarded

#### Scenario: Redirects are not followed
- **WHEN** the gateway answers a forwarded request with a redirect
- **THEN** the redirect is returned to the browser and not followed by the command

### Requirement: humpyard ui can keep the admin credential out of the browser
With `--credential-env NAME`, `humpyard ui` SHALL read the admin credential from that environment variable and add it to every forwarded request, so the credential never reaches the browser. Because anyone who can reach its port then acts with that credential, it SHALL refuse a `--listen` address that is not a loopback address unless `--allow-remote-with-credential` is also given. Without `--credential-env` the browser supplies its own credential and any listen address is allowed.

#### Scenario: A credential and a public address
- **WHEN** `--credential-env` is given with `--listen 0.0.0.0:8787` and no `--allow-remote-with-credential`
- **THEN** the command exits with an error that explains why

#### Scenario: The credential stays out of the browser
- **WHEN** `--credential-env` is used
- **THEN** no response from the command contains the credential, and forwarded requests carry it

#### Scenario: No credential given
- **WHEN** `--credential-env` is not used and `--listen 0.0.0.0:8787` is given
- **THEN** the command serves on that address and the browser supplies its own credential
