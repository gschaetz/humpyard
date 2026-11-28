# Design

## Context

See proposal.md (Why). Constraints that shape the approach:

- ADR 0032: the GUI is a separate process from the gateway, embedded in the same binary; the gateway serves no page.
- ADR 0004: provider keys reach the gateway only as environment variables named in the configuration; the loader refuses an inline key.
- ADR 0025: the GUI keeps no configuration of its own. Provider keys are credentials, not configuration, but the rule that the GUI adds no second home for settings still holds.
- ADR 0027: the admin API is opened only by a named credential the file grants scopes to; there is no config include mechanism, so a credential's hash must be in the one file the gateway loads.
- ADR 0036: nothing here may add work to the request path. `up` adds none: the gateway child is the same `serve`.

## Goals / Non-Goals

**Goals:** one command from install to a signed-in, visible gateway on one machine; the smallest possible secret store; no change to `serve`, `ui` or the request path.

**Non-Goals:** editing routes, targets or prices from the page; storing any secret other than a provider key; a keychain, a service installer or Windows support; remote or multi-user use of `up`.

## Decisions

### 1. `up` is a supervisor that serves the GUI in its own process and runs `serve` as a child

`up` builds the GUI's router with the same function `humpyard ui` uses (so headers, Host and Origin rules, the token exchange and the `/admin/v1` forwarder are the same code), adds a `/local` router, and spawns `humpyard serve --config <run>/config.toml` as a child (the same executable, found with `current_exe`). The gateway is therefore a separate process that serves no page, as ADR 0032 requires; `up` is the third thing, and exists only on a single machine.

*Alternatives:* two children (`serve` and `ui`) with `up` only supervising — the `/local` routes need to write the key file and restart the gateway, which belongs to the supervisor, so a child GUI would need a back-channel to it. A GUI inside the gateway — rejected by ADR 0032 and nothing here changes its reasons.

### 2. The state directory, and what is in it

`$XDG_CONFIG_HOME/humpyard` or `~/.config/humpyard` (also on macOS, so the path is one the docs can state), created `0700`: `config.toml` (written by the `init` flow, owned by the user, never modified by `up`), `keys.env` (only if the user saves a key; `0600`; `NAME=value` lines), `ledger.db`, and a per-run `run/<pid>/` directory (`0700`) holding the generated config. `--state-dir` overrides it.

### 3. The admin credential is per run and only its hash is on disk

At start `up` creates a random credential, computes its hash, and writes `run/<pid>/config.toml`: the user's config text plus an `[admin.credentials.up]` block (scope `read`; the GUI of this change only reads the admin API) holding the hash. The gateway child loads that file. The credential itself stays in `up`'s memory, is added to forwarded requests by the same code as `ui --credential-env` (so the browser never holds it), and is gone on exit; the run directory is removed on exit and on the next start if a crash left one. If the user's file already has `[admin]`, `up` refuses (no TOML merging, no surprises) and says to run `serve` and `ui` separately.

*Alternative:* store a long-lived admin credential in the state directory — rejected: it would be the second stored secret, and a per-run one costs nothing since the browser is opened by `up`.

### 4. The browser is opened signed in with the existing token

`up` generates the per-run access token `ui` already supports and opens `http://127.0.0.1:<port>/#token=…`. The page reads the fragment, exchanges it once at `/ui/session` (the token is in a request body from then on), and clears the fragment from the address with `history.replaceState`. A fragment never reaches a server or a `Referer`. If opening a browser fails, `up` prints the address and the token's one-time link.

### 5. The key store: one file, read by `up`, passed as environment

`keys.env` is read only by `up` (a `keystore` module with no other caller; the architecture test pins that). At start and on each change, `up` builds the child's environment as: the file's values, overlaid by `up`'s own environment (so the environment wins). A key change from the page: validate the key live against the provider (the same check `init` uses, with the same request budget), write the file (temp file in the same directory created `0600`, `fsync`, rename), then restart the child. The restart is a brief interruption of a single user's gateway, and avoids a new "reload the environment" path in the gateway. If the new child fails to start with the new key, `up` restores the previous file and child and reports the failure.

`GET /local/keys` returns per provider `{present, source}` only. `POST /local/keys` and `DELETE /local/keys/{provider}` return no key and no echo of one. The provider list is the configuration's `api_key_env` names, so the page cannot create a variable the configuration does not name. Values are held in a `Secret` newtype whose formatting shows nothing (the one `ui` already has), and are zeroed after use where cheap.

### 6. The `/local` surface

`GET /local/connect` (the live address and the configuration's route names, no secret), `GET|POST|DELETE /local/keys…`, `POST /local/test` (sends the fixed prompt `Say "ok".` to the gateway's loopback address, route `fast` if the configuration has it, else the first route, and returns the answering target, latency and outcome). All sit behind the same checks as the rest of the GUI (session cookie, loopback Host, no cross-site) plus a fixed `Content-Type: application/json` and a body limit. If `up` is told to listen on a non-loopback address, `/local/keys` answers `403 key_entry_disabled` and `/local/connect` still works.

### 7. First-run screens, front-loaded value

The home screen is the *Connect a tool* panel and the empty state until the first answered call, then the query view with the savings strip first. The panel's text comes from one table in `ui/src` that is checked against `docs/clients.md` by a test, so the page and the docs cannot drift. No gateway change: the strip already reads `GET /admin/v1/stats?baseline=`; "first answered call" is read from the same stats call.

### 8. Ports and the `init` flow

`up` uses the ports `serve` and `ui` use today (`127.0.0.1:8080` and `127.0.0.1:8787`), with `--listen` and `--ui-listen` to change them; a taken port is a refusal with the flag to use, not a silent hop (a hopped port would break the tool settings a user has already copied). The first-run configuration is the `init` flow called as a library function with the providers whose keys it finds; with none, `up` serves the GUI with a setup state and starts the gateway when a key is saved.

## Threat model

Assets: provider API keys (the one new stored secret); the per-run admin credential and access token; the gateway's availability (a key change restarts it). Actors: the user; any local process of the same user (out of scope: it can read the file as the user can); any other local user; any web page the user visits; a network attacker (loopback only).

Trust boundaries crossed: browser to `/local` (a new write surface on a local web service); `up` to the filesystem (the first stored secret); `up` to its child (environment); `up` to a provider (key validation).

| Threat | Mitigation | Enforced by |
|---|---|---|
| A web page the user visits posts a key to `/local/keys` or makes the browser read status | Session cookie (`HttpOnly`, `SameSite=Strict`) required; cross-site and cross-origin requests refused; JSON content type required (no simple-form post) | the `ui` boundary tests extended to `/local` routes (`tests/up_local.rs`) |
| DNS rebinding reaches the local service | Host must be loopback or refused, as for `ui` | same tests |
| The key file is readable by another local user | Directory `0700`, file `0600`, created that way (never chmod after), atomic replace; `up` refuses to start if it finds the file or directory looser and says how to fix it | `tests/up_keystore.rs` (modes; refusal on a loose file) |
| A saved key is returned or logged | No call returns a key; `Secret` formatting shows nothing; request bodies on `/local/keys` are never logged; the key is never put on a command line (child env only) | log-capture test; response-shape test; a test that the child's argv has no key |
| `serve` or another command reads the file, widening who holds the secret | Only the `keystore` module reads it and only `up` calls it; `serve`'s loader still refuses an inline key and a missing variable | `tests/architecture.rs` rule; a `serve` test for the missing-variable failure; ADR 0004 tests unchanged |
| A malicious page enters a key of the attacker's (to make the user's traffic go to an account the attacker watches) | Same protections as the first row; a key is only accepted for a provider the configuration names | the `/local` tests |
| Another local user connects to the loopback ports | Loopback is not a user boundary: the GUI needs the per-run token, and the gateway needs the per-run admin credential for `/admin/v1`; the client path on loopback is unauthenticated as `init`'s config is today | **Gap, accepted:** a local user of a shared machine can send requests through the gateway. Documented: use `serve` with keys on a shared machine |
| A crashed run leaves the admin hash or run directory behind | The run directory holds a hash only, is `0700`, and is removed at the next start | `tests/up_run.rs` |
| A key change while a request is in flight drops it | The restart drains the child with the gateway's own shutdown grace; a single user is the only client | **Gap, accepted:** in-flight requests at the restart may fail; the page says it before restarting |
| The token in the opened URL leaks | In the fragment (never sent), exchanged once, removed from the address | UI test; `ui` token tests |
| The user's key is used by a child process that is not the gateway | The child is `current_exe serve` with a fixed argument list; no shell | `tests/up_run.rs` |

Gaps named above are the two marked *accepted*. `docs/threat-model.md` gets a new boundary (B12) for them.

## Risks / Trade-offs

- [A stored key is a new thing to protect, and the first] → a file only the user can read, the least that is useful, opt-in, write-only, and a new ADR that names the limits so they are not widened casually.
- [`up` is a fourth way to run humpyard] → it creates no new behavior in `serve` or `ui`; it composes them, and the docs lead with it for a single machine and say what to use otherwise.
- [A restart on every key change] → one user, a rare action, a stated interruption; a reload path in the gateway for its environment would be a larger change to the process that serves traffic.
- [macOS-first assumptions about paths and `open`] → the state directory and browser opening are behind two small functions; Linux uses `xdg-open`, and failure prints the link. Windows is out.

## Migration Plan

New command, no change to existing ones. Deploy: release with `up`; README quick start leads with `humpyard up`. Rollback: a user stops using `up`; `config.toml` is a normal configuration that `serve` loads, and the key file is deleted by removing the state directory or by the page's remove action.

## Open Questions

- Whether `init` and `up` should share one first-run function or `up` should call `init`'s as a library (the tasks start with the second and move if the seam is awkward).
- The exact name of the home screen's tabs; it does not change a spec.
