# Proposal

## Why

The first real-provider smoke test (OpenCode Go) failed with HTTP 400: the provider requires an
`x-opencode-session` header. Providers commonly need extra static headers (session or app
identifiers, custom user agents, routing hints). Config currently has no way to send them.

## What Changes

- Add an optional `headers` table to each provider; its entries are sent on every call to that provider.
- Reject headers that would override authentication (`authorization`, `x-api-key`) and invalid names or values at config load.

Out of scope: per-request or per-session header values (for example forwarding the client's
session id as `x-opencode-session`). Switchyard 0.3.0's client only supports static headers per
backend, so a conversation-scoped value is a follow-up that needs a different client mechanism.

## Capabilities

### New Capabilities

### Modified Capabilities
- `gateway-config`: providers accept a `headers` table, validated at load.
- `provider-pool`: configured headers are sent on every call to the provider.

## Impact

- `src/config.rs` (schema and validation), `src/pool.rs` (pass headers to the backend), docs and example config.
- Backward compatible: the key is optional.
