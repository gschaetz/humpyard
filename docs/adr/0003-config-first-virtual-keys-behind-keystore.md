# 0003. Config-first virtual keys behind a `KeyStore` trait

- Status: accepted
- Date: 2026-10-07 (recorded retroactively; decided 2026-10-07)
- Supersedes: none
- Change: [add-cost-tracking](../../openspec/changes/archive/2026-10-07-add-cost-tracking)

## Context

Per-client budgets need client identity. Options were virtual keys declared in config, keys managed
in a database through an admin API (as LiteLLM does), or a single global budget. The owner expects
database-managed keys to be the long-term direction, but an admin API needs its own authentication,
cache invalidation and migration story, which the first cost-tracking change should not carry.

## Decision

Clients authenticate with gateway-issued keys declared in config by stable **id** and SHA-256
**hash**, behind an async `KeyStore` trait with a config-backed implementation. `keygen <id>` mints
a key once and prints its config block. With no keys configured the gateway is open and unbudgeted.
Everything downstream (budget counters, ledger rows, policy, key info) uses the key **id**, never
the key or its hash.

A later change adds database-managed keys: a `keys` table using the same hash scheme, a
database-backed store with a short-TTL cache, an admin API protected by an admin key, and an
`import-keys` command to seed it from config. The ledger needs no migration.

## Consequences

- Changing keys or limits needs a restart until the managed store exists.
- Unsalted SHA-256 is adequate only for high-entropy random keys, so `keygen` is the only
  documented way to create one.
- Code must keep depending on the trait and on ids; that is invariant 10 in
  [invariants.md](../invariants.md).
