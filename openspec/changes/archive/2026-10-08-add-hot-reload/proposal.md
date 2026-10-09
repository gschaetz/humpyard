# Proposal

## Why

Routing rules, routes, targets and keys change often, and every change currently needs a restart:
streams are cut at the grace period, endpoint health is forgotten and, for a gateway in front of
agent harnesses, that is disruptive. Customer-defined routing makes frequent edits the normal case.

## What Changes

- On SIGHUP the gateway re-reads its config file and, only if the whole file loads, validates and
  builds, atomically swaps in a new snapshot of everything derived from the config: providers,
  targets, routes, selector rules, keys, budget limits, health settings and the budget policy.
- An optional `reload_poll_secs` makes the gateway also re-read the file on a timer and reload
  when its content (not its timestamp) changed, for places where nothing can send SIGHUP, such as
  a distroless container in Kubernetes picking up an updated ConfigMap. A bad file is attempted
  once, not on every tick.
- In-flight requests (including streams) finish on the snapshot they started with.
- A bad file never takes effect: the previous config keeps serving and the error is logged and
  reported.
- Carried over across a reload: endpoint health for endpoints that did not change, budget
  counters, the usage ledger, and the session state of routes whose definition did not change.
- Not reloadable, refused with a clear message: the listen address, the ledger path,
  `shutdown_grace_secs`, `reload_poll_secs`, and switching between keyless and keyed operation. Provider API keys are
  re-read from the process environment, which a running process cannot change.
- `GET /v1/health` reports the loaded config's fingerprint and load time and the last reload's
  outcome, so operators can confirm a reload took.
- README and docs updated, including how to reload in Docker, Kubernetes and launchd.

Out of scope: inotify-style file watching, a reload API call (needs the admin credential), reloading the
listen address or ledger, rotating provider keys without a restart.

## Capabilities

### New Capabilities
- `config-reload`: how the running gateway picks up a changed config.

### Modified Capabilities

## Impact

`src/server.rs` (snapshot, run, handlers), `src/server/explain.rs`, `src/pool.rs` (reusing
breakers), `src/routing.rs` (reusing route instances), `src/budget.rs` (adjustable threshold),
`src/main.rs` (SIGHUP), tests, docs, README, ADR 0012.
