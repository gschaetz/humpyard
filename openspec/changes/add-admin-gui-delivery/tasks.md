# Tasks

## 1. The GUI project

- [ ] 1.1 Create `ui/` (Preact, TypeScript, Vite) with a committed lockfile, a client generated from `docs/openapi/admin-v1.json`, a sign-in step that keeps the credential in `sessionStorage`, and one screen showing gateway health and the contract-version check; the build emits no inline scripts or styles; verify with a type-check, unit tests of the client and the version check, and a production build whose `index.html` has no inline script
- [ ] 1.2 Add a CI job for `ui/` (`npm ci --ignore-scripts`, type-check, tests, build, `npm audit`) that runs on pull requests and uploads `dist`; verify the job passes and fails when a generated client no longer matches the contract

## 2. Serving it from the gateway

- [ ] 2.1 Add the `ui` Cargo feature that embeds `ui/dist`, a `build.rs` that writes a placeholder page when `ui/dist` is missing, and a static-asset module that serves embedded files by exact path with the single-page fallback and the headers in the spec (Content-Security-Policy, `nosniff`, `no-referrer`, `no-cache` on `index.html`, immutable on hashed files); verify tests for a file, the fallback, a missing asset, traversal in several encodings, each header, and that a build without `ui/dist` succeeds and serves the placeholder
- [ ] 2.2 Add `[admin.ui] enabled` to the configuration (rejected without an admin credential, unknown keys rejected) and mount the asset module at `/admin/ui/` on the listener that serves `/admin/v1`, deciding per request from the current configuration so a reload turns it on or off; verify tests that it is 404 when off or absent, served when on, on the admin listener only when `[admin] listen` is set, and toggled by a reload with no restart
- [ ] 2.3 Add the `humpyard ui` command: serve the same files and forward exactly `/admin/v1/*` to `--url` (no redirects followed, bounded time and size, same headers), `--credential-env` injection, and the loopback rule with `--allow-remote-with-credential`; verify tests with a fake gateway for forwarding, a non-admin path answered 404, the credential added upstream and never in a response, redirects returned and not followed, and the refusal on a public address
- [ ] 2.4 Register the new modules in the architecture rules (the asset module reachable from the server; the `ui` command module only from `main`, like `ctl`); verify the architecture tests pass and a test fails when `serve` references the command module

## 3. Building and releasing it

- [ ] 3.1 Build the GUI in the release workflow before the Rust builds and embed it, and add a Node stage to the `Dockerfile`; verify a release-style build and the image serve the GUI (not the placeholder) at `/admin/ui/` and that `docker build` still works from a clean checkout

## 4. Docs and wrap-up

- [ ] 4.1 Document the GUI in a new `docs/admin-gui.md` (the two ways to run it, the security model, TLS and loopback advice, Docker and Kubernetes as a second container running `humpyard ui`), `docs/configuration.md` and `docs/deployment.md`, and add it to the nav; verify the strict docs build
- [ ] 4.2 Update `docs/architecture.md` (the module table, the status table, the "Last updated" line, the operator-surface diagram) and verify its Mermaid diagrams still match the code
- [ ] 4.3 Accept ADR 0032 when the design is confirmed; verify `docs/adr/README.md` lists it as accepted
- [ ] 4.4 Run fmt, clippy `-D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --document-private-items`, the full test suite and the `ui/` checks, then archive this change with `openspec archive add-admin-gui-delivery --yes`; verify all gates pass and the spec deltas merge

## Workflow follow-up

- The screens that matter, each its own change on this delivery: a models view (per target: requests, failure and response-signal rates, cost, latency, endpoint state), decisions (filters, each failed attempt), overrides (list, create, remove).
- Release notes: new, off by default, nothing changes for existing deployments.
