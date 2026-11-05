# 0032. The admin GUI is one set of files, served by the gateway when enabled and by a separate command otherwise

- Status: proposed
- Date: 2026-11-05
- Supersedes: none (refines [0025](0025-the-ui-has-no-state-of-its-own.md): where it said the GUI is a separate client, this says what separate means for delivery)
- Change: `openspec/changes/add-admin-gui-delivery`

## Context

ADR 0025 says the GUI holds no state of its own and is a separate client of the admin API, and leaves its hosting open. ADR 0027 says the admin API is opened only by a named credential the file grants scopes to, on the main listener or its own. Most people who run humpyard do not run Kubernetes: a GUI that needs a service mesh, a second web server or a CORS setting in front of the gateway is a GUI they will not use, and a browser app cannot reach an API on another origin without one of those.

## Decision

There is one admin GUI, built from `ui/` (Preact, TypeScript, Vite) with a client typed from the published OpenAPI document, and it is delivered in two ways from the same embedded files. With `[admin.ui] enabled = true` (off by default; needs an admin credential configured) the gateway serves it at `/admin/ui/` on the listener that serves `/admin/v1`, from files embedded in the binary and looked up by exact path, with a strict Content-Security-Policy. The command `humpyard ui` serves the same files on its own address and forwards exactly `/admin/v1/*` to a gateway, optionally adding the admin credential itself so the browser never holds it, and refusing a non-loopback address in that case unless told otherwise. The GUI keeps its credential for the browser session only and checks the API's contract version on load.

"Separate client" is kept in the sense that matters: the GUI's code is separate, uses only the published admin API, has no state of its own, and the gateway knows nothing about what it shows. What changes is that delivery may be bundled, never by default. Serving the files grants nothing; every data call is authorised by the admin API as before.

Alternatives considered: only a separate deployment (needs infrastructure many users lack, and CORS or a proxy); the gateway serving it by default (a web page on every deployment's port); CORS on the admin API plus any static host (a new security-sensitive setting, and the credential must live in the browser); a server-rendered UI service holding the credential (fits one operator, not several named admins, and needs its own login).

## Consequences

A single-binary user turns on one setting and opens a browser; a Docker or Kubernetes user runs the same image a second time with `humpyard ui`; a hardened deployment leaves it off and serves no web page at all. There is one GUI to build and test, protected by the contract test. The cost is a Node step in CI and the release builds (never needed for development, because a plain `cargo build` serves a placeholder), a few hundred KB of embedded files, and an npm dependency tree to keep small and audited. A separate `humpyard ui` can be a different version from its gateway, so the GUI warns on a contract mismatch. The admin credential crosses the network in a header: TLS, loopback, or a private admin listener is the deployment's job, and the documentation says so.
