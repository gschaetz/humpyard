# 0012. The configuration is an atomically swapped snapshot, reloaded on SIGHUP

- Status: accepted
- Date: 2026-10-08
- Supersedes: none
- Change: [add-hot-reload](../../openspec/changes/archive/2026-10-08-add-hot-reload)

## Context

Routing rules, routes, targets and keys change often, and every change required a restart that
cuts running streams and forgets endpoint health. Rules, routes, selectors, targets and keys
reference each other, so reloading only some of them could leave a mixed state that never existed
in any file.

## Decision

Everything derived from the config lives in one immutable snapshot held behind a lock. A request
takes the current snapshot once and uses it to the end; a reload builds a complete new snapshot
off to the side (read, validate, build) and swaps it in a single step, or does nothing and reports
an error. SIGHUP triggers it. Where nothing can send a signal (a distroless container, Kubernetes), an
optional poll (`reload_poll_secs`) re-reads the file on a timer and reloads when its content hash
changed; a failing file is attempted once per change. State that describes unchanged things is carried over by identity:
endpoint health for unchanged endpoints, session state for unchanged routes; the ledger and budget
counters are process-wide. Settings that cannot change in a live process (listen address, ledger
path, shutdown grace, keyless-versus-keyed) make the whole reload refuse with a clear message.

Alternatives considered: reloading selected sections only (mixed states, more code paths); inotify-style
file watching (symlink-swap ConfigMaps, extra dependency, partial writes; a content-hash poll
gets the benefit without those problems); an admin API call (needs the
admin credential that does not exist yet; can be added on top later); restarting with draining
(still resets health and cuts long streams).

## Consequences

Edits apply without dropping traffic and a typo cannot take the gateway down. Two requests that
straddle a reload may see different configs, each internally consistent. Rotating provider API
keys still needs a restart because they come from the process environment. A future admin API can
trigger the same reload function.
