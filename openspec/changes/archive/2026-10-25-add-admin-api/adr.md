# ADR Review

## Reviewed

ADR-0025 The UI has no state of its own (operations are expiring overrides, database keys are bounded by profiles,
the GUI is a client of the admin API); ADR-0012 The configuration is an atomically swapped snapshot (credentials
follow it); ADR-0004 No plaintext secrets (hashes only, ids in logs); ADR-0016 Metrics are one registry with
bounded labels (the new counter has a fixed `result` label); ADR-0003 Config-first virtual keys behind a `KeyStore`
trait (client keys stay as they are). The rest of 0001 to 0026 are unaffected.

## Outcome

One durable commitment qualifies: administrative authority is granted by the configuration to named, scoped
principals (a credential first, later possibly identity-provider sign-in), kept separate from client keys and never held in the database. Later work (database keys, the GUI, config writes)
attaches to it as scopes, so it is recorded now. Tactical choices (the override kind for revocation, the route
table, `admin:<id>` in audit rows, the hand-written OpenAPI document) stay in `design.md`.

## New ADRs

- [ADR-0027 Admin authority is granted by configuration to named, scoped principals, never to a client key or a database row](../../../docs/adr/0027-admin-authority-is-granted-by-configuration-to-named-scoped-principals.md) (proposed; supersedes none)
