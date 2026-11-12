# The admin GUI: `humpyard ui`

`humpyard ui` serves a web page for looking at a running gateway. It is a client of the
[admin API](api.md), like [`ctl`](ctl.md), and it runs as its own process: the gateway itself serves no web
page and gains no setting. The page is built into the `humpyard` binary and the container image, so there is
nothing else to install.

The first screen is the **query view**: a query bar, a chart of one metric over time (or as a ranking), and a table
of the groups it covers, drawn from `GET /admin/v1/stats`, `GET /admin/v1/stats/series` and `GET /admin/v1/health`. It
only reads. More views follow as separate changes ([ADR 0032](adr/0032-the-admin-gui-runs-as-its-own-process-from-the-same-binary.md),
[ADR 0035](adr/0035-the-admin-apis-time-series-are-read-from-the-decision-log-at-query-time.md)).

## The query view

**The query bar** chooses what is shown: a window (1 hour, 6 hours, 24 hours, 7 days or 30 days), a grouping (route,
target or endpoint), a metric (failure rate, fallback rate, p90 latency or requests), optional route and target
filters (applied when you press Enter or leave the field), whether to draw the metric over time or as a ranking, and
how often to refresh. It starts from 24 hours, by target, on failure rate, refreshing every minute. Changing any of
them reads again at once.

**The chart** draws one line per group (the eight busiest) across the window. Each bucket is as wide as the window
needs (a minute for an hour, 15 minutes for 24 hours, two hours for 7 days). A bucket in which a group had no calls is
a **gap**, not a zero, and its hover text says so; hovering any point gives its time (UTC), value and request count, so a
spike on three requests is not mistaken for a trend. The last bucket is still filling and is drawn dashed. If the
gateway read only the newest 50,000 decisions of the window, the part of the chart before the earliest one it counted is
shaded and labelled, because there it does not know and a quiet line would be wrong. Series are told apart by marker
shape and a label at the end of the line as well as by colour. Clicking a series in the legend (or a bar in the
ranking) **filters the table to that group**; clicking it again shows all.

**The ranking** shows the same metric as bars, one per group, from the window's totals, in the table's order. A group
with nothing to measure shows "no value" and no bar.

**The table** leads with four figures: health (for targets and endpoints; a route has no endpoint state of its own),
requests, failure rate, fallback rate and p90 latency. The arrow at the left of a row opens the rest: answered calls,
tokens, average cost, p50 and p99, and the four response-signal rates (a dash where no call was observed, never 0%).
By default the groups that need attention come first:

1. groups whose endpoints are all cooling down;
2. then groups with any endpoint that is not healthy;
3. then by failure rate, then fallback rate, then requests, then name;
4. groups that had no calls last.

A rate is ranked by the **lower end of its 95% confidence interval**, so a rate on few calls ranks below the same rate
on many: one failed call out of one does not outrank thirty out of a hundred. The cells show the raw rates, and
clicking a column header sorts by them; "Default order" returns to the list above.

**The address is the query.** The choices are kept in the part of the page address after `#`, so a view can be
bookmarked and sent to a colleague, who sees the same query once they have signed in. The fragment never reaches a
server and holds no credential or token. It is read against fixed lists: an unknown window, grouping, metric or refresh
period is replaced by the default, and a filter is only ever shown as text.

**Refreshing.** The view re-reads the chart, the table and the health together on the chosen period (off, 30 seconds, 1
minute or 5 minutes) and shows "Updated hh:mm:ss". It reads only while the page is visible, and once more when you
return to a page whose data is older than the period. It never starts a request while the previous one is running. After
a failure it waits the period, then twice that, up to five minutes, and returns to the period after a success; the
earlier numbers stay on screen but are dimmed and marked out of date with the time they are from. A gateway that is
busy with another statistics read ([heavy reads](api.md#heavy-reads)) says so, and is not asked again before its
`Retry-After`. If the gateway refuses the credential, refreshing stops and the sign-in is shown. With the period off,
nothing is read until you change the query or press "Refresh now".

**Limits.** At most eight series are drawn (the table and the filters reach every group). A window longer than the
50,000 newest decisions shows the shaded region above. Each open page that is visible refreshes on its own, so many
open pages add up on the gateway; the statistics reads are limited to one at a time per surface and shared for ten
seconds, so they cannot slow the requests the gateway serves, but a busy answer is possible. Figures can be up to ten
seconds old.

**An older gateway.** `humpyard ui` can run at a different version from its gateway. A gateway without the series
endpoint answers 404; the page then shows the table and a note that the gateway cannot draw a series, with the
version warning as before.

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

### The query view

The threat model of `add-gui-query-view` was checked against what was built; each row is held by a test:

| Threat | Where it is held |
|---|---|
| A huge window, tiny interval or many groups exhausts the gateway | parameters bounded and refused before any read (`tests/admin_reads.rs`), the 50,000-row cap, the heavy-read gate and shared result (`tests/heavy_reads.rs`), the series in the isolation test's load (`tests/isolation.rs`) |
| A credential without `read` reads the series; a client key reads cross-key aggregates | the route table scope and the 404 on `/v1/stats/series` (`tests/admin_reads.rs`) |
| A parameter reaches SQL unescaped | bound parameters, integers parsed from windows and intervals, unknown parameters refused (hostile values in `tests/admin_reads.rs`) |
| A group name injects script through the chart, legend, tooltip or ranking | text nodes only, the raw-HTML lint rule, hostile-name tests in `chart.test.tsx`, `bars.test.tsx` and `query-view.test.tsx` |
| A crafted or shared link | fixed lists and text-only filters, nothing credential-like read or written (`query.test.ts`, `query-view.test.tsx`) |
| A truncated or quiet series read as "quiet" | `truncated` and `covered_since_ms`, the shaded region, gaps for null buckets (`chart.test.tsx`, `series.rs` tests) |
| The chart weakens the policy | classes and attributes only; `check-dist` in the `ui` job; the header test in `tests/ui.rs` |
| Open or many tabs refresh forever and load the gateway | `refresh.ts` tests with fake timers for each rule; the series and the statistics asked one after the other because the gate allows one heavy read at a time (a test fails if they overlap) |
| Stale numbers look current; a refused credential is retried forever | out-of-date marking with the time, late answers dropped, refresh stops on 401 or 403 (`refresh.test.ts`, `query-view.test.tsx`) |

**Not closed:** the admin API has no per-credential rate limit, so a stolen or shared `read` credential can read as fast
as it likes (bounded cost per call only), and many visible pages add up on the gateway. It is recorded as a gap in the
[threat model](threat-model.md).

Accepted residual risks: malware running as your user can read the environment and the browser regardless;
the browser-held credential is a long-lived secret typed into a form with no second factor (sign-in through an
identity provider is the real fix, [ADR 0027](adr/0027-admin-authority-is-granted-by-configuration-to-named-scoped-principals.md));
and there is no browser-level end-to-end test yet.
