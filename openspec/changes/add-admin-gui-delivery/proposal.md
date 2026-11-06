# Proposal

## Why

The admin API is complete enough for a GUI (config with provenance, decisions with each failed attempt, statistics with response-signal rates, health, quota, overrides, credentials, and a published OpenAPI contract). ADR 0025 decided that the GUI is a separate client of that API, and left its stack and hosting open. Hosting matters most for a public project: most people who run humpyard do not run Kubernetes, and a GUI that needs a service mesh or a second web server in front of the gateway is a GUI they will not use. A browser app also needs a same-origin path to `/admin/v1` (or CORS) to reach the gateway at all. The safest place for that path is not inside the gateway that serves every model request.

## What Changes

- **The GUI runs as its own process, from the same binary.** A new command, `humpyard ui --url <gateway>`, serves the GUI on its own address (loopback by default) and forwards exactly `/admin/v1/*` to a gateway. It is a client command like `ctl`: `serve` never reaches it. The gateway serves no GUI, so a deployment that does not run the command has no web page, no static-file code path and nothing new in the serving process.
- **Hardened for the cases that go wrong.** The command answers only requests whose `Host` is a loopback name or one given with `--allow-host` (defeating DNS rebinding), rejects cross-site and cross-origin requests, normalises paths before forwarding, and can add the admin credential itself (`--credential-env`) so it never reaches the browser, in which case it refuses a non-loopback address unless told otherwise.
- **A threat model drove the hardening** (in `design.md`): it found that a loopback proxy holding the credential is open to other local users and to a mistyped or intercepted upstream, so that mode requires a per-run access token and an `https` upstream (unless the upstream is loopback).
- **The GUI is built from source in `ui/`** (Preact, TypeScript, Vite) with a client typed from `docs/openapi/admin-v1.json`, so the existing contract test protects it. It renders every API string as text, never markup (lint-enforced and tested with hostile strings), keeps its credential for the browser session only, and checks the API's contract version on load.
- **Release builds embed the built GUI and fail without it**, so a release can never ship the placeholder silently; a plain `cargo build` (feature off) still works and serves a placeholder that says how to build the GUI.
- **First delivery is the mechanism plus the screen people ask for most:** a models view (per target: requests, failure and response-signal rates, cost, latency, endpoint state). Decisions and overrides are separate changes on top.
- **Serving the GUI from the gateway is deliberately not part of this change.** If it is ever wanted, it comes back as its own change, only on a dedicated `[admin] listen` address, never the main one. ADR 0032 records the decision and why.

## Capabilities

### New Capabilities

- `admin-gui`: what the GUI is and may do: the files and headers served, text-only rendering, the credential's lifetime, the contract-version check, and the models view.

### Modified Capabilities

- `cli`: `humpyard ui`, its forwarding, its credential handling and its cross-site and rebinding defenses.
- `release-packaging`: release builds embed the built GUI and fail without it.

## Impact

- New `ui/` project (a Node toolchain for the GUI only; `cargo build` needs none) and a CI job for it.
- `src/` gains an embedded-assets module and a `ui` command module that only `main` may use, registered in the architecture rules.
- `Dockerfile` and `.github/workflows/release.yml` build the GUI first and embed it, with a smoke test that fails on the placeholder.
- Docs: a GUI page, `deployment.md` (Docker and Kubernetes as a second container running `humpyard ui`), `architecture.md`.
- No change to the gateway's serving path, the admin API or its contract, the ledger, or client traffic.
- Not in scope: gateway-side serving, decisions and overrides screens, config patches (ADR 0025's patch generation), database-managed keys, CORS, an identity-provider login, and serving custom assets from disk.
