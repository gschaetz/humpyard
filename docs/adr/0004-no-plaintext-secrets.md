# 0004. No plaintext secrets in config, logs or the ledger

- Status: accepted
- Date: 2026-10-07 (recorded retroactively; decided 2026-10-06)
- Supersedes: none
- Change: [add-core-gateway](../../openspec/changes/archive/2026-10-06-add-core-gateway), [add-provider-headers](../../openspec/changes/archive/2026-10-06-add-provider-headers), [add-cost-tracking](../../openspec/changes/archive/2026-10-07-add-cost-tracking)

## Context

The repository is public and the gateway handles two kinds of secret: provider API keys and the
virtual keys it issues. The legacy modelrelay config stores provider keys in plaintext, which
we do not want to inherit.

## Decision

Provider keys are read from environment variables named in config (`api_key_env`); an inline key is
a startup error. Client keys exist in config only as SHA-256 hashes; a field holding a plaintext key
is a startup error naming the key id. Neither keys nor hashes are written to logs, the ledger or
`Debug` output, and provider `headers` may not override authentication (`authorization`,
`x-api-key`). Callers are identified by key id everywhere.

## Consequences

- Operators manage environment variables (or a secret manager that exports them); `check-config`
  names any missing variable.
- The modelrelay migration must convert keys to env references and never copy plaintext
  ([0007](0007-one-shot-modelrelay-migration.md)).
- Enforced by config tests, Debug-redaction tests and log-capture tests (invariant 8 in
  [invariants.md](../invariants.md)).
