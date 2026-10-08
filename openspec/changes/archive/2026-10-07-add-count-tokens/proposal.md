# Proposal

## Why

Claude Code and other Anthropic-format clients call `POST /v1/messages/count_tokens` to measure
context size (for the context display and for deciding when to compact). humpyard answers 404, so
those clients get an error. Switchyard only forwards this call to Anthropic-protocol backends and
refuses it otherwise, and none of our providers has a count endpoint (they speak OpenAI Chat), so
there is nothing to forward to.

## What Changes

- Add `POST /v1/messages/count_tokens`, answering in Anthropic's shape (`{"input_tokens": n}`).
- The number is a **local estimate** of the prompt tokens the routed model would see: no upstream
  call, no cost, no ledger entry. A response header marks it as an estimate.
- Same authentication, route allowlist and model checks as `/v1/messages`; it keeps working when a
  key's budget is exhausted, because it costs nothing.
- The estimator errs high (a conservative bytes-per-token ratio calibrated against what real
  providers report), because overshooting context is worse than compacting a little early.

Out of scope: exact tokenization per provider, a "probe" mode that asks the provider for its real
count, and the Responses API equivalent (`/v1/responses/input_tokens`).

## Capabilities

### New Capabilities

### Modified Capabilities
- `gateway-api`: adds the token-counting endpoint.

## Impact

- New module for the estimator; a handler in `server.rs`; docs (`docs/clients.md` no longer lists
  the gap). No new dependencies.
