# Design

## Context

ADR 0025 decides that the GUI holds no state of its own and is a separate client of the admin API; ADR 0027 decides that admin authority is a named credential granted scopes by the file, that it opens only `/admin/v1`, and that the API's contract is published and tested. Observed in the code: `[admin]` has `listen` (the admin API on its own address) and `credentials`; the admin routes are one table in `src/server/admin.rs` checked against `docs/openapi/admin-v1.json`; `ctl` is the existing pattern for a client command the serving path never reaches (an architecture rule enforces it); the `Dockerfile` and the release workflow run only `cargo build --release`, so a web build is a new step in both.

Humpyard is a public project and most of its users will not run Kubernetes. The hosting decision ADR 0025 left open has to work for a single binary on a VM, for Docker or Compose, and for Kubernetes, without a service mesh, a second web server or a new CORS setting, and without adding anything to the process that serves every model request.

In-force ADRs read: 0004 and 0013 (no content stored), 0016 (metrics registry), 0017 (strict routes), 0025, 0027, 0030 (response signals, shown by the first screen).

## Goals / Non-Goals

**Goals:**
- A user can open the GUI with no infrastructure beyond humpyard itself.
- The gateway's serving process gains nothing: no web page, no static-file path, no new setting.
- The GUI stays a client of the published admin API; nothing in the gateway knows what it shows.
- The one new network service (`humpyard ui`) is hardened against the ways a local web service is attacked.

**Non-Goals:**
- Serving the GUI from the gateway (see decision 1), CORS, an identity-provider login, serving custom assets from disk.
- The decisions and overrides screens, config patches, database-managed keys: separate changes.
- Changing the admin API or its contract.

## Decisions

**1. The GUI runs as its own process, from the same binary; the gateway serves no GUI.** `humpyard ui` serves the embedded files and forwards `/admin/v1/*`. It is one command beside `ctl`, in the binary and the container image already shipped, so there is no second artifact; a Docker or Kubernetes user runs the same image a second time with a different command. Serving the GUI from the gateway was considered and deferred: it is only a convenience over one extra command, and it puts a web page, a static-file path and reload logic into the process that handles all traffic, on a listener that is the internet-facing one by default. If it is wanted later it comes back as its own change, only on a dedicated `[admin] listen` address, using the same files and build.

**2. Source in `ui/`: Preact, TypeScript, Vite.** Small output (a few hundred KB), no runtime dependency beyond the browser, and a client generated from `docs/openapi/admin-v1.json`, so a contract change that breaks the GUI breaks its build and the existing contract test protects both. Alternatives: Rust/WASM (one language, a heavier toolchain, fewer contributors), a server-rendered UI service (holds the credential server-side but needs its own login and fits several named admins badly).

**3. Embedded by a Cargo feature that is off by default.** The `ui` feature embeds `ui/dist` into the binary. With it off, `humpyard ui` serves a placeholder and `cargo build` needs no Node. With it on and `ui/dist` missing, `build.rs` fails the build, so the release workflow and the `Dockerfile` (which turn the feature on) cannot ship a placeholder; a smoke test of the built binary adds a second line of defence. Files are looked up by exact path in the embedded set with no filesystem access at request time, so traversal has nothing to reach.

**4. Forwarding.** The command normalises the request path (decoding percent-encoding, collapsing dot segments and repeated slashes) before matching, and forwards only a path still under `/admin/v1`. It forwards the method, the body and a small allow-list of headers, drops hop-by-hop headers, does not follow redirects, and bounds time and size. With `--credential-env` it replaces any `Authorization` the browser sent.

**5. Defending a local web service.** A service on `127.0.0.1` is reachable by any page the user visits: through DNS rebinding the page can make its origin resolve to loopback and talk to the service as same-origin, and with `--credential-env` that would act with the admin credential. So the command accepts a request only when its `Host` is loopback or one named with `--allow-host`, answers 403 to `Sec-Fetch-Site: cross-site` and to a state-changing request whose `Origin` is not its own, and refuses to start on a non-loopback address without an `--allow-host`. The browser-held credential mode needs these too, but is less exposed because the attacker page does not hold the credential.

**6. Headers.** A Content-Security-Policy with `default-src 'self'`, `script-src 'self'`, `style-src 'self'`, `connect-src 'self'`, `frame-ancestors 'none'`, `base-uri 'none'`, plus `nosniff` and `Referrer-Policy: no-referrer`. The Vite build emits no inline scripts or styles, so the policy needs no `unsafe-*`; this constrains later library choices (a library that injects style tags is out).

**7. Data is rendered as text.** Decisions and statistics carry strings that other principals influence: the requested model name, session ids, provider error text. The GUI renders every API string as text; a lint rule bans raw-HTML insertion and a component test renders hostile strings and asserts that no element is created from them. Preact escapes by default, so this is a guard against someone adding the unsafe call later.

**8. Credentials.** In the default mode the user pastes an admin credential, kept in `sessionStorage` only (not `localStorage`, not a cookie). With `--credential-env` the browser holds none. The documentation says plainly that the credential crosses the network in a header, so a non-loopback admin API needs TLS.

**9. Build and release.** A Node job builds the GUI (`npm ci --ignore-scripts`, type-check, tests, lint, `vite build`, `npm audit`); the release workflow builds it before the Rust builds and the `Dockerfile` gains a Node stage. Supply chain: a committed lockfile, no install scripts, a handful of dependencies.

**10. Contract version check.** The GUI reads `info.version` from `/admin/v1/openapi.json` on load and compares it with the set it was built for; it warns and still shows data, so a slightly newer or older gateway is not a dead end. This matters for `humpyard ui` because it can run at a different version from its gateway.

**11. First delivery is the mechanism plus the models view.** The skeleton, the typed client, sign-in, the version check, and one screen: per target, the figures people ask for (requests, failure and fallback rates, response-signal rates over observed calls, tokens, cost, latency, endpoint health) from `/admin/v1/stats` and `/admin/v1/health` only. Decisions and overrides follow as separate changes.

## Risks / Trade-offs

- **`humpyard ui` is a small forward proxy and a security-sensitive one.** Mitigated by exact-prefix forwarding after normalisation, no redirects, the Host and Origin rules, loopback by default, and tests for each.
- **A credential pasted into a browser form** has no logout and no second factor; `--credential-env` removes the exposure for one operator, and an identity-provider login is the real fix later (ADR 0027).
- **Build complexity**: a Node step in CI, the `Dockerfile` and the release builds. Contained: development never needs it, and a release cannot silently omit the GUI.
- **The strict policy limits chart and component libraries** to ones that emit no inline styles or scripts.
- **No browser-level end-to-end test** in this change: type checks, component tests (including hostile strings) and the proxy tests in Rust. A smoke test that loads the page in a real browser can follow.
- **`cargo install` and distribution packages** build without the GUI unless they run the Node build; they serve the placeholder and say how to build.
- **Version skew** between `humpyard ui` and its gateway, handled by the contract check.

## Open Questions

None blocking. The default address `127.0.0.1:8787` is chosen here; changing it before implementation is cheap.
