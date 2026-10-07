# Design

## Context

Greenfield crate. Switchyard 0.3.0 crates on crates.io already provide a neutral request/response
IR (`switchyard-protocol`) and codecs between OpenAI Chat, OpenAI Responses and Anthropic Messages,
including stream events (`switchyard-translation`). See `docs/background.md` for findings.

## Goals / Non-Goals

**Goals:** a request path that later phases can insert routing into without reshaping; correct
streaming; minimal config.

**Non-Goals:** routing, multiple upstreams, auth, persistence, OpenAI Responses endpoint.

## Decisions

- **Reuse Switchyard IR and codecs** instead of hand-written serde structs. Decode inbound to the IR,
  encode the IR for the upstream. Alternative: own types per provider; rejected as duplicated work
  and it would diverge from the libsy `Request` that phase 2 needs.
- **Pipeline as a function over the IR**: handler decodes, calls `dispatch(ir_request) -> stream of
  IR events`, then encodes in the client's format. Phase 2 inserts routing between decode and dispatch.
- **Upstream protocol fixed to OpenAI Chat** in this phase (broadest compatibility with free/local
  providers). Anthropic-native upstreams are a later change.
- **axum + reqwest (rustls, stream)**; SSE via axum's `Sse` with the translation crate's per-event
  encoder; dropping the response body drops the reqwest stream, which cancels the upstream call.
- **TOML via serde with `deny_unknown_fields`**; key read from an env var at startup.
- **Errors**: one internal error enum, rendered per-endpoint (OpenAI vs Anthropic error shape).
- **Tests**: a mock upstream (axum on an ephemeral port) for streaming, error and disconnect cases.

## Risks / Trade-offs

- Switchyard crates are 0.x and may change → pin exact versions, wrap behind one `translate` module.
- Codec gaps for unusual fields (tools, reasoning) → pass through via IR preservation where
  supported, add fixture tests with real captured payloads.
- Rust 1.96.1 MSRV may exceed some contributors' toolchains → `rust-toolchain.toml` pin.

## Open Questions

- Whether to expose the OpenAI Responses endpoint (Codex clients) soon after this change.
