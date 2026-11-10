# The admin GUI: `humpyard ui`

`humpyard ui` serves a web page for looking at a running gateway. It is a client of the
[admin API](api.md), like [`ctl`](ctl.md), and it runs as its own process: the gateway itself serves no web
page and gains no setting. The page is built into the `humpyard` binary and the container image, so there is
nothing else to install.

The first screen is the **models view**: for each target, its requests, failure and fallback rates, tokens,
cost, latency and endpoint health, drawn from `GET /admin/v1/stats` and `GET /admin/v1/health`. It only reads.
More views follow as separate changes ([ADR 0032](adr/0032-the-admin-gui-runs-as-its-own-process-from-the-same-binary.md)).

## Run it

The gateway needs an admin credential with the `read` scope (see [the admin API](api.md)). Then point the
command at the gateway:

```sh
humpyard ui --url https://gateway.example
# humpyard ui: the admin GUI is at http://127.0.0.1:8787
```

Open that address in a browser and paste the admin credential into the page. It is kept in the browser's
`sessionStorage` for that tab only (never `localStorage` or a cookie) and sent as a header on each request.

To keep the credential out of the browser, let the command hold it:

```sh
export HUMPYARD_ADMIN_KEY=...                  # read from the environment only, never an argument
humpyard ui --url https://gateway.example --credential-env HUMPYARD_ADMIN_KEY
# access token (shown once; the page asks for it): ...
```

The page asks for the access token the command printed, which it exchanges for a cookie that is `HttpOnly`
and `SameSite=Strict`. Anyone who can reach the port acts with the credential, so the token is required in this
mode and a new one is made on every start.

| Mode | Where the credential is | Use it when |
|---|---|---|
| Browser-held (default) | pasted into the page, kept for the tab | several people share a host, or each person has a credential of their own |
| `--credential-env` | the command's environment, never the browser | one operator on their own machine |

In the shared-credential mode every action is recorded as that one `admin:<id>`, so a team that needs to tell
people apart should use the browser-held mode with one credential per person.

## Options

| Option | Meaning |
|---|---|
| `--url` | the gateway whose `/admin/v1` is forwarded (required): `https://gateway.example` or `http://127.0.0.1:8080` |
| `--listen` | where to serve the GUI, default `127.0.0.1:8787` |
| `--credential-env NAME` | read the admin credential from this environment variable and add it to every forwarded request |
| `--allow-host HOST` | a host name this service answers to besides loopback (repeatable), like `ui.internal:8787`; required to listen on a non-loopback address |
| `--allow-remote-with-credential` | allow `--credential-env` with a `--listen` address that is not loopback |
| `--allow-insecure-upstream` | with `--credential-env`: allow a plain `http://` gateway on another machine |

The command refuses to start on a mistake that would expose the credential: a non-loopback `--listen` with
no `--allow-host`, `--credential-env` with a non-loopback `--listen` unless you say so, and `--credential-env`
with a plain `http://` gateway on another machine unless you say so. The address the credential is sent to
is printed at start, and it is sent nowhere else.

## What it forwards, and what it refuses

`humpyard ui` serves its own embedded files by exact path and forwards **only** `/admin/v1/*` to `--url`.
Paths are checked before they are matched: a dot segment (`..`, `%2e%2e`, `..%2f`) is refused, not resolved.
Redirects from the gateway are returned, not followed, and time and size are bounded.

A web page on another site can try to reach a service on `127.0.0.1` (for example by pointing its own name at
your loopback address). So a request is answered only when:

- its `Host` is a loopback address or one named with `--allow-host`;
- it is not marked `Sec-Fetch-Site: cross-site`;
- a request that changes something (not `GET` or `HEAD`) does not carry an `Origin` that is not its own.

Every response carries a `Content-Security-Policy` with `default-src 'self'` and no inline script or style, so
the page loads nothing from anywhere else, plus `X-Content-Type-Options: nosniff` and
`Referrer-Policy: no-referrer`. Every string from the API (model names, session ids, provider errors) is shown
as text; some of them are chosen by whoever holds a client key.

## Where to run it

- **On your machine.** `humpyard ui --url ...` and open `http://127.0.0.1:8787`. Nothing else to configure.
- **Beside a gateway on a VM.** Run it on the VM and reach it through an SSH tunnel
  (`ssh -L 8787:127.0.0.1:8787 host`), which keeps it on loopback. If it must be reached by name, set
  `--listen` and `--allow-host` and put TLS in front of it (the page and the access token cross the network).
- **Docker.** The image has the GUI in it (from 0.29.0). Run it a second time with a different command:

    ```sh
    docker run -d --name humpyard-ui -p 127.0.0.1:8787:8787 \
      ghcr.io/gschaetz/humpyard:0.29.0 \
      ui --url https://gateway.example --listen 0.0.0.0:8787 --allow-host localhost:8787
    ```

    Inside the container `--listen` has to be non-loopback for the published port to work, which is why
    `--allow-host` is required; the `-p 127.0.0.1:` prefix keeps it off the network.
- **Kubernetes.** A second container (or Deployment) running the same image with the `ui` command, reached
  through `kubectl port-forward` or an ingress with TLS. Give it `--url` of the gateway's service. Do not set
  `--credential-env` on something others can reach.

## TLS and the credential

The admin credential crosses the network in a header, so an admin API that is not on loopback needs TLS (the
gateway speaks plain HTTP; see the [threat model](threat-model.md)). With `--credential-env` the command
requires `https://` for a gateway on another machine for this reason.

## Version skew

The page reads `info.version` from `/admin/v1/openapi.json` and compares it with the contract it was built
for. If they differ it shows a warning and still shows data, so a `humpyard ui` that is a release behind or
ahead of its gateway keeps working.

## Building from source

A plain `cargo build` or `cargo install humpyard` builds **without** the GUI: `humpyard ui` then serves a
placeholder page that says so. Release binaries and the container image are built with it. To build it
yourself:

```sh
cd ui && npm ci --ignore-scripts && npm run build && cd ..
cargo build --release --features ui
```

With the `ui` feature on and `ui/dist` missing, the build fails rather than ship a placeholder. The release
workflow and the container image run `scripts/smoke-ui.sh` on the finished binary, which fails when the page
is the placeholder.

## Threat model check

The threat model for this change is in its [design](https://github.com/gschaetz/humpyard/blob/main/openspec/changes/add-admin-gui-delivery/design.md)
and was checked against what was built. Every mitigation is in the code and each has a test that fails without
it:

| Threat | Where it is held |
|---|---|
| DNS rebinding to loopback | the Host rule and the refusal to listen publicly without `--allow-host` (`tests/ui.rs`) |
| Another local user using the injected credential | the per-run access token and cookie (`tests/ui.rs`); use the browser-held mode on a shared host |
| The credential sent to the wrong party, or in clear text | `--url` only, printed at start; `https` required off loopback (`tests/ui.rs`) |
| A path that leaves `/admin/v1` | normalisation before matching (`src/ui/path.rs`, `tests/ui.rs`) |
| Cross-site requests (CSRF) | `Sec-Fetch-Site`, `Origin` and `SameSite=Strict` (`tests/ui.rs`) |
| Script injection from a string a client key chose | text-only rendering, a lint rule against raw HTML, a hostile-string component test, the CSP |
| Credential theft from the browser | `sessionStorage` only, CSP, no third-party script; `--credential-env` keeps it out entirely |
| The credential in logs, arguments or responses | environment variable only; a gateway that echoes it is not passed on (`tests/ui.rs`) |
| Framing by another site | `frame-ancestors 'none'` (`tests/ui.rs`) |
| A read-only credential acting | the admin API checks the scope on every call; the page only hides controls |
| A flood or an oversized answer | time and size bounds (`tests/ui.rs`) |
| A release without the GUI | the `ui` feature fails the build; the smoke test rejects the placeholder |
| A malicious dependency | pinned dependencies, a committed lockfile, `npm ci --ignore-scripts` and `npm audit` in CI |

Accepted residual risks: malware running as your user can read the environment and the browser regardless;
the browser-held credential is a long-lived secret typed into a form with no second factor (sign-in through an
identity provider is the real fix, [ADR 0027](adr/0027-admin-authority-is-granted-by-configuration-to-named-scoped-principals.md));
and there is no browser-level end-to-end test yet.
