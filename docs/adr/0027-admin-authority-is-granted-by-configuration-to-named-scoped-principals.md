# 0027. Admin authority is granted by configuration to named, scoped principals, never to a client key or a database row

- Status: proposed
- Date: 2026-10-25
- Supersedes: none
- Change: `openspec/changes/add-admin-api`

## Context

ADR 0025 plans an operator interface that is a client of an admin API, and database-managed client keys bounded by
profiles. It says a client key must never be able to mint keys, but today there is no admin credential at all:
operating the gateway means a client key with `read_config` or `manage_overrides`. More than one person will
administer this gateway, so authority has to be per person, auditable, expiring and revocable without a deploy,
and it has to stay out of reach of anything a client, or the database, can hold. Sign-in through an identity
provider (OIDC) is likely later, and should extend this model rather than replace it.

## Decision

Administrative authority is held by a **named principal that the configuration file grants scopes to**. The first
kind of principal is a **credential**: `[admin.credentials.<id>]`, with the hash of its secret, a closed set of
**scopes** (`read`, `overrides`, `credentials`, and later `keys` and `config_write`) and an optional expiry. It
opens only the versioned `/admin/v1` API, optionally on its own listener. It opens nothing else, and no client key
or database key ever opens it. A later kind may be a token from a trusted identity provider, accepted only when
the file names the provider and maps its identities or groups to scopes; the file still decides who may do what.

- **Separate from clients.** The admin and client authenticators never consult each other's credentials. A
  client key's flags (`read_config`, `manage_overrides`) keep working on the client endpoints and grant nothing
  on `/admin/v1`.
- **Never in the database.** Admin authority is not stored in the ledger, so a tampered or leaked ledger cannot
  create it, and database keys can never carry an admin scope, however profiles evolve. The file also solves the
  bootstrap: the first admin cannot come from anywhere else.
- **A principal is a string id,** recorded as `admin:<id>` in audit rows so it cannot be confused with a client
  key id. Revocation names a principal id, so a new kind of principal needs no change to how actions are audited
  or revoked.
- **Revocation without a deploy** is an expiring runtime override that only removes access, so it fits ADR 0025's
  rule that operations restrict and never widen. Editing the file and reloading remains the way to remove or
  change a principal permanently.
- **The contract is published.** The admin API has a hand-written OpenAPI document that a test keeps in step with
  the routes, so a GUI or any other client is typed against it.

*Alternatives that lost:* a single shared admin bearer, as `[metrics]` has (anonymous audit, and removing one
person rotates everyone's secret); an admin scope on ordinary client keys (the separation would rest on
convention, and a database key could in time inherit it); admin principals in the database (a leaked ledger would
grant authority, and nothing could bootstrap the first); and no revocation without a deploy (too slow for a
departing teammate).

## Consequences

Per-person admin access with a real audit trail, an all-keys read surface, and a clean place for key management
(`keys` scope, ADR 0025's database keys), later config writes and identity-provider sign-in to attach, each as one
more scope or one more kind of principal. Principals follow the configuration lifecycle, so a reload changes them
atomically. It costs a second authenticator to maintain and test, and a leaked admin secret lives until it is
revoked or removed from the file. We must keep true that nothing in the ledger can grant or widen admin
authority, that the two namespaces never fall through to each other, and that the OpenAPI document matches the
routes.
