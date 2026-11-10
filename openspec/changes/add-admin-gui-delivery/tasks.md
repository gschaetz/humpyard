# Tasks

## 1. The GUI project

- [x] 1.1 Create `ui/` (Preact, TypeScript, Vite) with a committed lockfile, a client generated from `docs/openapi/admin-v1.json`, a sign-in step that keeps the credential in `sessionStorage`, and the contract-version check; configure the build to emit no inline scripts or styles and add a lint rule that bans raw-HTML insertion; verify with a type-check, lint, unit tests of the client and the version check, and a production build whose `index.html` has no inline script
- [x] 1.2 Add the models view: per target, requests, answered calls, failure and fallback rates, response-signal rates over observed calls (no rate when none were observed), tokens, cost, latency percentiles and endpoint health, from `/admin/v1/stats` and `/admin/v1/health`, with a window selector and an error state that does not keep stale numbers; verify component tests including a hostile model name rendered as text with no element created, a target with no observed calls, and an unreachable API
- [x] 1.3 Add a CI job for `ui/` (`npm ci --ignore-scripts`, type-check, lint, tests, build, `npm audit`) that runs on pull requests and uploads `dist`; verify the job passes and fails when the generated client no longer matches the contract

## 2. The `humpyard ui` command

- [x] 2.1 Add the `ui` Cargo feature (off by default) that embeds `ui/dist`, with `build.rs` failing the build when the feature is on and `ui/dist` is missing, a placeholder page when it is off, and an embedded-assets module that serves files by exact path with the single-page fallback and the headers in the spec; verify tests for a file, the fallback, a missing asset, traversal in several encodings, each header, the placeholder with the feature off, and the build failure with the feature on and no files
- [x] 2.2 Add `humpyard ui --url URL [--listen ADDR]`: serve the assets and forward exactly `/admin/v1/*` after path normalisation (no redirects followed, bounded time and size, an allow-list of request headers, hop-by-hop headers dropped); verify tests with a fake gateway for forwarding, a non-admin path answered 404, traversal and encoded dot segments leaving the prefix answered 404, redirects returned and not followed, and the size and time limits
- [x] 2.3 Add `--credential-env`, `--allow-remote-with-credential`, `--allow-host` and the cross-site rules: Host accepted only when loopback or allowed, 403 for `Sec-Fetch-Site: cross-site` and for a non-GET/HEAD request with a foreign `Origin`, refusal to start on a non-loopback address without an allowed host, the credential injected in place of the browser's and never in a response; add the per-run access token (printed once, exchanged in a request body for an `HttpOnly`, `SameSite=Strict` cookie) and the `https` rule for a non-loopback upstream with `--allow-insecure-upstream`; verify tests for a rebound Host, a cross-site request, a foreign-origin POST, a named host, a public address with no allowed host, the refusal of a credential with a public address, a missing or wrong token, the token exchange, a cleartext public upstream, and that the credential never appears in any response, log line or URL
- [x] 2.4 Register the modules in the architecture rules (the embedded-assets module and the `ui` command module reachable only from `main`, like `ctl`); verify the architecture tests pass and one fails when the serving path references either module

## 3. Building and releasing it

- [x] 3.1 Build the GUI in the release workflow before the Rust builds, enable the `ui` feature in the release builds and add a Node stage and the feature to the `Dockerfile`; add a smoke test that runs the built binary's `humpyard ui` and fails when the page is the placeholder; verify a release-style build and the image serve the GUI, and that a build with the files removed fails

## 4. Docs and wrap-up

- [ ] 4.1 Document the GUI in a new `docs/admin-gui.md` (running `humpyard ui`, the credential modes, TLS and loopback advice, `--allow-host`, Docker and Kubernetes as a second container running the same image) and `docs/deployment.md`, and add it to the nav; verify the strict docs build
- [ ] 4.2 Update `docs/architecture.md` (the module table, the status table, the "Last updated" line, the operator-surface diagram) and verify its Mermaid diagrams still match the code
- [ ] 4.3 Check the threat model in `design.md` against what was built (each row still true, each proof exists) and record the result in `docs/admin-gui.md`; then accept ADR 0032 when the design is confirmed; verify `docs/adr/README.md` lists it as accepted
- [ ] 4.4 Run fmt, clippy `-D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items` (and with `--all-features` where `ui/dist` is built), the full test suite and the `ui/` checks, then archive this change with `openspec archive add-admin-gui-delivery --yes`; verify all gates pass and the spec deltas merge

## Workflow follow-up

- Each its own change on this delivery: the decisions view (filters, each failed attempt, response signals), the overrides view (list, create, remove), config patches (ADR 0025), and, only if asked for, serving the GUI from the gateway on a dedicated `[admin] listen` address.
- Release notes: a new command; nothing changes for existing deployments, and the gateway serves nothing new.
