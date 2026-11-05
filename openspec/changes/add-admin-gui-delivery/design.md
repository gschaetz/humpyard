# Design

## Context

ADR 0025 decides that the GUI holds no state of its own and is a separate client of the admin API; ADR 0027 decides that admin authority is a named credential granted scopes by the file, that it opens only `/admin/v1`, and that the API's contract is published and tested. Observed in the code: `[admin]` already has `listen` (the admin API on its own address) and `credentials`; the admin routes are one table in `src/server/admin.rs` checked against `docs/openapi/admin-v1.json`; `ctl` is the existing pattern for a client command that the serving path never reaches (an architecture rule enforces it); the `Dockerfile` and the release workflow run only `cargo build --release`, so a web build is a new step in both.

Humpyard is a public project and most of its users will not run Kubernetes. The hosting decision ADR 0025 left open has to work for a single binary on a VM, for Docker or Compose, and for Kubernetes, without a service mesh, a second web server or a new CORS setting.

In-force ADRs read: 0004 and 0013 (no content stored), 0016 (metrics registry), 0017 (strict routes), 0025, 0027, 0030 (response signals, shown by the first screens later).

## Goals / Non-Goals

**Goals:**
- A user can open the GUI with no infrastructure beyond humpyard itself.
- A deployment that does not want a web page on the gateway's port can keep it off and run the GUI separately.
- The GUI stays a client of the published admin API; nothing in the gateway knows what the GUI shows.
- Serving the GUI adds no data path and no authority.

**Non-Goals:**
- The screens beyond health (models, decisions, overrides, config patches): separate changes.
- CORS on the admin API, an identity-provider login, serving custom assets from disk.
- Changing the admin API or its contract.

## Decisions

**1. One GUI, two deliveries.** The gateway can serve the GUI (`[admin.ui]`) and `humpyard ui` can serve it beside a gateway that does not. Both serve the same embedded files, so there is one implementation to maintain. This relaxes ADR 0025's "separate client" from "separately deployed" to "separate code that uses only the published API", recorded in ADR 0032.

**2. Built-in is off by default and mounted on the admin listener at `/admin/ui/`.** It lives next to the API it needs (same origin, so no CORS and no proxy), moves with `[admin] listen` when that is set, and needs an admin credential to be configured at all. The setting is read from the current configuration on each request, so a reload turns it on or off without a restart. A `serve` with the setting off has no GUI route that answers anything but 404.

**3. Source in `ui/`: Preact, TypeScript, Vite.** Small output (a few hundred KB), no runtime dependency beyond the browser, and a typed client generated from `docs/openapi/admin-v1.json` (so a contract change that breaks the GUI breaks its build, and the existing contract test protects both). Alternatives: Rust/WASM (one language but a heavier toolchain and fewer contributors), server-rendered with a Rust service (the credential stays server-side, but it needs its own login in front and fits several named admins badly).

**4. Embedded by a Cargo feature, with a placeholder.** A `ui` feature (on in release builds) embeds `ui/dist` into the binary; `build.rs` writes a placeholder `index.html` when `ui/dist` does not exist so a plain `cargo build` never needs Node and never fails. Files are looked up by exact path in the embedded set; there is no filesystem access at request time, so path traversal has nothing to reach.

**5. Headers.** A Content-Security-Policy with `default-src 'self'`, `script-src 'self'`, `style-src 'self'`, `connect-src 'self'`, `frame-ancestors 'none'`, `base-uri 'none'`, plus `X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`. The Vite build is configured to emit no inline scripts or styles so the policy needs no `unsafe-*`. `index.html` is `no-cache`; hashed files are immutable.

**6. The page grants nothing.** The files need no credential and contain no gateway data, so serving them cannot disclose anything; every call goes to `/admin/v1` with the user's credential and is authorised there as before. The credential is kept in `sessionStorage` (cleared when the tab's session ends), never in `localStorage` or a cookie.

**7. `humpyard ui` is a forward proxy for exactly `/admin/v1/*`.** It forwards nothing else, follows no redirects, bounds time and size, and adds the same headers. With `--credential-env` it injects the credential, so the browser never holds it; because anyone who can reach the port then acts with it, a non-loopback `--listen` is refused unless `--allow-remote-with-credential` is given. Without `--credential-env` the browser brings its own credential and the address can be anything. It is a client command: a new module that only `main` may use, like `ctl`, registered in the architecture rules; `serve` reaches only the static-asset module.

**8. Build and release.** The GUI is built once per release by a Node job (`npm ci --ignore-scripts`, type-check, tests, `vite build`) whose `dist` is handed to the Rust builds; the `Dockerfile` gains a Node stage ahead of the Rust stage. The Node toolchain is needed only to change the GUI. Supply chain: a committed lockfile, `npm ci --ignore-scripts`, an audit step in CI, and dependencies kept to a handful.

**9. Contract version check.** The GUI reads `info.version` from `/admin/v1/openapi.json` on load and compares it with the set it was built for. It warns and still shows data, so a slightly newer gateway is not a dead end. This matters most for `humpyard ui`, whose version can differ from the gateway's.

**10. First delivery is the mechanism and one screen.** The skeleton, the typed client, the health screen, the embedding, `[admin.ui]` and `humpyard ui`. The screens people want (models with response-signal rates and cost, decisions, overrides) are separate changes that add views to a delivery that already works.

## Risks / Trade-offs

- **A web page on the gateway's port.** Mitigated by off by default, the admin listener, embedded-only files, CSP, and no data in the page. Anyone who can reach the port sees the login page and learns that the admin API exists; the same is true of `/admin/v1` answering 401.
- **A credential over cleartext HTTP.** The admin credential travels in a header. Documented: use TLS, or `humpyard ui` on loopback with `--credential-env`, or keep the admin listener on a private address.
- **The credential in the browser.** `sessionStorage` is readable by any script on the origin, hence the strict CSP and no third-party scripts; `humpyard ui --credential-env` removes the exposure for a single operator.
- **Build complexity.** A Node step in CI, the Dockerfile and the release builds. Contained: development never needs it.
- **Binary size.** A few hundred KB with the feature on; the feature can be turned off for a minimal build.
- **Version skew** between a separate `humpyard ui` and its gateway, handled by the contract check.
- **Supply chain.** An npm dependency tree enters a Rust project; kept small, pinned and audited.

## Open Questions

None blocking. The URL path `/admin/ui/` and the setting name `[admin.ui] enabled` are chosen here; changing them before implementation is cheap.
