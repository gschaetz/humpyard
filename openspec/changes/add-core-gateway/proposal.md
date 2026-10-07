# Proposal

## Why

Nothing in the repo runs yet. Every later piece (Switchyard routing, provider pool, budgets)
needs a working request path that accepts client protocols, forwards to an upstream, and streams
the answer back. Building it first, without routing, gives us a testable foundation.

## What Changes

- Add an axum HTTP server exposing OpenAI Chat Completions (`/v1/chat/completions`), Anthropic
  Messages (`/v1/messages`), and `/v1/models`, plus `/healthz`.
- Decode inbound requests into Switchyard's neutral IR (`switchyard-protocol`) and encode them for
  one configured OpenAI-compatible upstream (`switchyard-translation`).
- Stream upstream SSE back to the client in the client's own protocol, and return buffered JSON
  for non-streaming requests.
- Add a TOML config file and a `clap` CLI (`serve`, `check-config`) to define the listener and
  one upstream with its API key (read from an env var, never inline).
- Structured request logging via `tracing`.

Out of scope: Switchyard routing, multiple providers/failover, health checks, budgets/keys,
`.modelrelay.json` import, auth on the inbound side.

## Capabilities

### New Capabilities
- `gateway-api`: inbound endpoints, accepted protocols, and client-visible error behavior.
- `upstream-proxy`: forwarding to the configured upstream, streaming, and upstream error mapping.
- `gateway-config`: config file format, CLI commands, and secret handling.

### Modified Capabilities

## Impact

- New Rust code in `src/`; `Cargo.toml` gains axum, tokio, reqwest, clap, serde, toml, tracing,
  and the Switchyard crates `switchyard-protocol` and `switchyard-translation` (0.3.0, Apache-2.0).
- Requires Rust 1.96.1+ (Switchyard MSRV, edition 2024).
- No existing specs are modified.
