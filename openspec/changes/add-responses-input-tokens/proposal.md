# Proposal

## Why

OpenAI's Responses API has a documented token-counting call, `POST /v1/responses/input_tokens`,
the Responses-side twin of the Anthropic `count_tokens` endpoint we already answer. SDK users and
agent frameworks that talk Responses can call it; humpyard answers 404. A real Codex CLI session
did not call it (verified 2026-10-08), but it is a public part of the protocol and costs almost
nothing to support with the estimator that already exists.

## What Changes

- Add `POST /v1/responses/input_tokens`, answering `{"object": "response.input_tokens",
  "input_tokens": n}` in OpenAI's shape, with the same local estimate, header and rules as
  `count_tokens`: no upstream call, no cost, no ledger entry, available to a key over budget,
  same authentication, allowlist and model checks, errors in the Responses/OpenAI error shape.
- Deliberately **not** adding `POST /v1/responses/compact`: it returns OpenAI-specific encrypted
  "compaction" items that only OpenAI's own models can produce, and the one client that uses it
  (Codex) does not call it for custom providers, which default to local compaction through ordinary
  `/v1/responses` calls. Recorded in design.md with how to revisit.

Out of scope: exact tokenization, stateful `previous_response_id` (still rejected, as for creates).

## Capabilities

### New Capabilities

### Modified Capabilities
- `gateway-api`: adds the Responses token-counting endpoint.

## Impact

`src/server.rs` (the existing counting handler becomes protocol-aware), tests, docs. No new
dependencies.
