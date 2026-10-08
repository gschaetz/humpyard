# Design

## Context

`main` serves with `axum::serve(listener, router)` and no shutdown handling. The ledger writer is a
background task fed by a bounded queue, so entries still queued (or in a request being cancelled)
are lost on exit. `axum::serve` spawns each connection as a detached task, so dropping the server
future does not cancel in-flight handlers. See proposal.md - Why and ADR 0005.

## Goals / Non-Goals

**Goals:** a clean, bounded, observable shutdown that never loses recorded spend.
**Non-Goals:** zero-downtime restarts, readiness probes, reload.

## Decisions

- **A `run` entry point owns the lifecycle.** `server::run(config, listener, signals)` builds the
  router and accounting (sharing one builder with the existing `router*` functions, which stay
  as they are for embedders and tests), serves with axum's graceful shutdown, and returns when the
  process may exit. `main` only turns OS signals into messages on an `mpsc` channel: the first
  message starts the drain, a second ends the wait. Making the signals a channel keeps `run`
  testable without sending real signals.
- **Hard stop via a watch flag, not task aborts.** After the grace period (or a second signal) a
  `watch` flag flips. Streaming responses select on it and end with an error event, so their usage
  tap drops and records the stream as cancelled; non-streaming handlers are dropped by a
  `select!` in the request entry point. We then wait up to 2 seconds for connections to wind down.
  Alternative considered: aborting connection tasks, which axum does not expose.
- **Record on drop.** `CallContext` gets a `Drop` that completes any entries it still holds, so a
  request cancelled mid-flight (shutdown or a client disconnect, which axum handles by dropping
  the handler future) still records the calls it already paid for. The normal paths drain these
  buffers first, so this adds nothing in the success case.
- **Flush is a marker through the queue.** The writer takes a message enum (`Entry` or `Flush`
  carrying a reply channel); on `Flush` it writes everything received before it, then replies.
  `Ledger::flush(&self)` therefore works through shared references, needs no ownership of the
  ledger, and preserves ordering. The existing consuming `shutdown` stays.
- **Bounded waits and exit codes.** Flush waits at most 10 seconds; failure exits non-zero and
  logs the writer's `failed` and `dropped` counters. A clean drain, including one that hit the
  grace period, exits 0 as long as the ledger was flushed.
- **Config**: `shutdown_grace_secs`, 0 to 3600, default 30.

## Risks / Trade-offs

- A handler cancelled by `select!` drops its upstream call mid-flight; the provider may still bill
  for it. Unavoidable, and the gateway records what it saw.
- Unix signals only (SIGTERM). Ctrl-C works everywhere; Windows has no SIGTERM, which is fine for
  a service run under a unit manager on Linux/macOS.
- The 10 second flush wait is a constant, not config, until someone needs to tune it.

## Migration Plan

Behavior change only at shutdown. Existing configs keep working (new key optional).
