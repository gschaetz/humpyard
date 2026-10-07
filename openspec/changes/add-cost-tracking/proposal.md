# Proposal

## Why

The gateway routes well but cannot say who used what or limit it. Cost control is the point of
routing cheap-first: operators need per-client keys, a record of tokens and dollars spent, and
budgets that stop or degrade traffic. The routing-policy seam exists to receive exactly this.

## What Changes

- **Virtual keys**: clients authenticate with gateway-issued keys defined in config by SHA-256
  hash (never plaintext). Each key has a stable id, optional route allowlist and budgets. With no
  keys configured the gateway stays open and unbudgeted, as today.
- **Usage ledger**: every upstream call, including routing-time judge calls, is recorded in SQLite
  (key, route, target, provider, model, tokens, cost) by an asynchronous writer off the request
  path. Streamed responses are recorded when the stream ends.
- **Pricing**: each endpoint declares USD-per-million-token prices; cost is computed from the
  tokens the provider reports.
- **Budgets**: per-key daily and monthly limits in USD and tokens (UTC periods), enforced from
  live in-memory counters hydrated from the ledger at startup.
  - **Restricted**: past a configurable fraction of any limit, expensive targets become ineligible
    through the routing policy, and tiers degrade as already specified.
  - **Exhausted**: requests fail with HTTP 402, or, per key, continue on zero-priced targets only.
- `GET /v1/key/info` (budget state for the calling key) and a `keygen` CLI command.

Out of scope: a database-managed key store and admin API (designed for, see design.md),
per-team hierarchies, provider health feeding the policy, token estimation when a provider reports
no usage, spend alerts.

## Capabilities

### New Capabilities
- `client-auth`: virtual keys, authentication on the inference endpoints, route allowlists, key info, key generation.
- `usage-ledger`: recording usage and cost for every upstream call, pricing, persistence and restart recovery.
- `budget-enforcement`: budget periods and limits, restricted and exhausted behavior.

### Modified Capabilities
- `gateway-config`: keys, ledger and endpoint prices.
- `routing-policy`: the policy context includes the authenticated key and its budget state.

## Impact

- New dependencies: `sqlx` (SQLite), `sha2`; new modules for auth, pricing, ledger and budget.
- Request path gains authentication before routing and a usage tap on responses (buffered and streamed).
- Config grows `[keys.*]`, `[ledger]`, `[budget]` and per-endpoint `price`; the `endpoints` entries
  stay backward compatible when no key has a money budget.
