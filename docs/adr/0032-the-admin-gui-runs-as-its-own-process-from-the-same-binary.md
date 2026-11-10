# 0032. The admin GUI runs as its own process from the same binary

- Status: accepted
- Date: 2026-11-05
- Supersedes: none (refines [0025](0025-the-ui-has-no-state-of-its-own.md): where it said the GUI is a separate client, this says where it runs and what protects it)
- Change: `openspec/changes/add-admin-gui-delivery`

## Context

ADR 0025 says the GUI holds no state of its own and is a separate client of the admin API, and leaves its hosting open. ADR 0027 says the admin API is opened only by a named credential the file grants scopes to, on the main listener or its own. Most people who run humpyard do not run Kubernetes: a GUI that needs a service mesh, a second web server or a CORS setting in front of the gateway is a GUI they will not use, and a browser app cannot reach an API on another origin without one of those. The gateway is also the process that handles every model request, and its main listener is the internet-facing one in many deployments.

## Decision

There is one admin GUI, built from `ui/` (Preact, TypeScript, Vite) with a client typed from the published OpenAPI document, embedded in the binary and served only by a separate command, `humpyard ui`, which runs as its own process. The command serves the embedded files by exact path under a strict Content-Security-Policy and forwards exactly `/admin/v1/*` to a gateway, after normalising the path and without following redirects. It can add the admin credential itself so the browser never holds it; it then requires a per-run access token (exchanged for an `HttpOnly`, `SameSite=Strict` cookie), refuses a non-loopback address unless told it is safe, and sends the credential only to the named gateway, over `https` unless that gateway is on loopback or the operator says otherwise. Because a local web service is reachable by any page the user visits, it accepts a request only when its `Host` is loopback or explicitly allowed, and refuses cross-site and cross-origin requests. The gateway serves no GUI and gains no setting for one.

The GUI renders every string from the API as text, keeps its credential for the browser session only, and checks the API's contract version on load. Release builds embed the built GUI and fail without it; a plain `cargo build` serves a placeholder.

Serving the GUI from the gateway was considered and deferred. It would save one command, at the price of a web page, a static-file path and reload logic in the process that serves all traffic, on the listener many deployments expose. If it is wanted later it is its own decision: only on a dedicated `[admin] listen` address, from the same embedded files.

Alternatives considered: only a separate deployment of a separate artifact (a second thing to download, version and host); CORS on the admin API plus any static host (a new security-sensitive setting, and the credential must live in the browser); a server-rendered UI service holding the credential (fits one operator, not several named admins, and needs its own login).

## Consequences

A single-binary user runs `humpyard ui`; a Docker or Kubernetes user runs the same image a second time with that command; a hardened deployment runs nothing and serves no web page. There is one GUI to build and test, protected by the contract test. The cost is a Node step in CI and the release builds (never needed for development), a few hundred KB of embedded files, an npm dependency tree to keep small and audited, and one new network service, `humpyard ui`, which must stay hardened: the Host and Origin rules, path normalisation and loopback default are part of the decision, not details. A separate `humpyard ui` can run at a different version from its gateway, so the GUI warns on a contract mismatch. The admin credential crosses the network in a header: TLS or loopback is the deployment's job, and the documentation says so.
