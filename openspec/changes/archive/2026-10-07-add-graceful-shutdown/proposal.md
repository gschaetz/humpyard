# Proposal

## Why

`humpyard serve` stops abruptly: on SIGTERM or Ctrl-C the process exits with requests in flight
and usage entries still queued for the ledger. Spend recomputed after a restart is then low, and
budgets (which are rebuilt from the ledger) silently under-count. ADR 0005 records this as
outstanding work. A gateway that meters money must not lose the record when it is stopped.

## What Changes

- On SIGINT or SIGTERM the server stops accepting connections and lets in-flight requests finish,
  for at most `shutdown_grace_secs` (default 30). A second signal ends the wait immediately.
- Requests still running when the grace period ends are cancelled; streams end with an error
  event and are recorded as cancelled with the usage seen so far.
- Before the process exits, every queued ledger entry is written (bounded wait), then it exits 0.
  If the ledger cannot be flushed it exits non-zero and logs how many entries may be lost.
- Usage already spent by a request that is cancelled (client disconnect or shutdown) is recorded
  too, instead of being dropped with the request.
- New config key `shutdown_grace_secs`.

Out of scope: draining on config reload, zero-downtime restarts, readiness endpoints.

## Capabilities

### New Capabilities
- `server-lifecycle`: how the server starts draining, how long it waits, and how it exits.

### Modified Capabilities
- `usage-ledger`: entries are written before a graceful exit; cancelled requests still record spend.
- `gateway-config`: the grace period setting.

## Impact

- `src/main.rs` (signals), `src/server.rs` (`run`, hard-stop handling), `src/ledger.rs` (flush),
  `src/metering.rs` (record on drop), `src/config/` (new key).
- No new dependencies. `server::router*` keep working unchanged for embedders and tests.
