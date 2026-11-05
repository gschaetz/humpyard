# Proposal

## Why

The admin API is complete enough for a GUI (config with provenance, decisions with each failed attempt, statistics with response-signal rates, health, quota, overrides, credentials, and a published OpenAPI contract). ADR 0025 decided that the GUI is a separate client of that API, and left two things open: its stack and how it is hosted. The hosting question matters most for a public project: most people who run humpyard do not run Kubernetes, and a GUI that needs a service mesh or a second web server in front of the gateway is a GUI they will not use. A browser app also needs a same-origin path to `/admin/v1` (or CORS) to reach the gateway at all, and that is the part that has to be answered before any screen is worth building.

## What Changes

- **One GUI, two ways to serve it, the same files either way.**
  - **Built in, off by default:** `[admin.ui] enabled = true` makes the gateway serve the GUI at `/admin/ui/` on the listener that serves `/admin/v1`. Same origin, no proxy, no extra process: `serve`, then open a browser.
  - **Separate, opt-out of the built-in one:** `humpyard ui --url <gateway>` is a new command (a client, like `ctl`; `serve` never reaches it) that serves the same files on its own address and forwards `/admin/v1` to a gateway. It is how the GUI is run next to a gateway that has the built-in one off, in a second container, or on a laptop.
- **The GUI is built from source in `ui/`** (Preact, TypeScript, Vite) with a client typed from `docs/openapi/admin-v1.json`, so the existing contract test protects it. Release binaries and the container image embed the built files; a plain `cargo build` without them still works and serves a placeholder that says how to build the GUI.
- **Safe by construction.** The gateway serves only embedded files, found by exact path, with a strict Content-Security-Policy; the page itself holds and discloses nothing, and every data call carries the admin credential and is authorised by the admin API exactly as before. The credential is kept for the browser session only. `humpyard ui` can add the credential itself (`--credential-env`) so it never reaches the browser, and then refuses to listen on a non-loopback address unless told it is safe.
- **First delivery is the mechanism plus one screen** (gateway health). The screens that matter (models, decisions, overrides) are separate changes on top of this one.
- A new ADR refines ADR 0025: the GUI is still a separate client with no state of its own; what changes is that it may be delivered inside the binary, off by default.

## Capabilities

### New Capabilities

- `admin-gui`: what is served, where, with which headers; what the page may and may not do; the compatibility check against the API's contract version.

### Modified Capabilities

- `gateway-config`: the `[admin.ui]` setting.
- `cli`: `humpyard ui`.
- `release-packaging`: release builds embed the built GUI; development builds without it still build.

## Impact

- New `ui/` project (Node toolchain for the GUI only; `cargo build` needs none) and a CI job for it.
- `src/` gains a static-asset module the admin router mounts when enabled, a `ui` command module, and `[admin.ui]` in `src/config/`.
- `Dockerfile` and `.github/workflows/release.yml` build the GUI first and embed it.
- Docs: a GUI page, `configuration.md`, `deployment.md` (Docker and Kubernetes as a second container running `humpyard ui`), `architecture.md`.
- No change to the admin API or its contract, to the ledger, or to client traffic. Nothing is served unless `[admin.ui]` is enabled.
- Not in scope: the screens beyond health, config patches (ADR 0025's patch generation), database-managed keys, CORS, an identity-provider login, and serving custom assets from disk.
