# Design

## Context

See proposal.md for why. Constraints from the code and the in-force ADRs:

- **ADR 0025** fixes the rules for any operator interface: it keeps no configuration state, structural change is a
  patch to the file, operations are expiring overrides, and the GUI is a separate client of this API. Database
  keys (later) are credentials bounded by profiles; nothing in the database may grant admin authority.
- **ADR 0012**: configuration is an atomically swapped snapshot reloaded on SIGHUP, so anything defined in
  `[admin]` follows the same lifecycle for free.
- **ADR 0004**: no plaintext secrets in config, logs or the ledger. Credentials are hashes; ids are what appears.
- **Precedent:** `[metrics]` already has `bearer_sha256` and an optional `listen`, with the rule that its
  credential "opens nothing else". The admin API follows the same shape.
- Decisions and statistics take their key filter as a parameter (`parse(params, key)`), and overrides are a store
  with kinds, expiry and audit rows keyed by a `created_by` string. Both can be reused.

## Goals / Non-Goals

**Goals:** per-person admin identity with scopes, expiry and revocation without a deploy; an all-keys read
surface; override management with a real creator; a published contract a GUI can be typed against; no effect on
a gateway that does not configure it.

**Non-Goals:** the GUI, CORS, key management, patch generation and config writes, per-profile rate limits,
`ctl` support, changing `read_config` / `manage_overrides`, and any new dependency.

## Decisions

### 1. Named credentials in the file, with scopes `read`, `overrides`, `credentials`

```
[admin]
listen = "127.0.0.1:9090"            # optional
[admin.credentials.gary]
sha256 = "sha256:..."
scopes = ["read", "overrides", "credentials"]
expires = "2027-01-01T00:00:00Z"     # optional
```

The id is the audit identity, and a *principal* is a string id with scopes granted by the file: a credential is
the first kind, and a token from a trusted identity provider (OIDC) can be a later one, mapped to scopes by
configuration, without changing how actions are audited or revoked (decision 5 and ADR 0027). Scopes are a closed set checked at load, and a later `keys` or `config_write` scope
adds a name without changing the format. Revoking another credential is its own scope, not part of `overrides`:
it is the one operation that can lock out colleagues, so it is granted on purpose.

*Alternatives:* a single admin bearer like `[metrics]` (anonymous audit rows, and removing one person rotates
everyone's secret); a scope flag on ordinary client keys (the separation ADR 0025 wants would rest on convention,
and a database key could in time inherit it); credentials in the database (a leaked or edited ledger could then
create admin access, and there would be no bootstrap).

### 2. A separate namespace, `/admin/v1`, optionally on its own listener

The admin routes are one router, merged into the main one or, when `[admin] listen` is set, served by a second
listener built the way the metrics listener is. Absent credentials mean no routes at all (404). Authentication is
a function of the admin snapshot only: it never consults client keys, and the client authenticator never consults
admin credentials, so neither can fall through to the other.

*Alternative:* `/v1/admin/...` shares the client prefix and invites a shared middleware. Rejected.

### 3. Reuse the existing handlers' cores through a caller-neutral seam

Config view, health, quota and overrides already build their JSON from `Snapshot` and the stores, with
authentication at the edge. The admin handlers call the same functions after their own authentication.
Decisions and statistics already take `key: Option<String>`; the client endpoint passes the caller's id, the admin
endpoint passes the `key` query parameter or nothing. One implementation per view keeps the two surfaces from
drifting, and the response shapes stay identical to the client endpoints'.

### 4. Revocation is a row in the overrides table, kept out of `Kind` and hidden from planning and from clients

A revocation is stored in the overrides table with its own row kind, `revoke_admin_credential`, and the credential
id as its subject. That reuses expiry, the maximum, audit rows and removal. It only removes access, so it fits
ADR 0025's rule that overrides restrict and never widen. It is deliberately **not** a variant of the `Kind` enum
that planning matches on: it has its own type and its own list inside the store, so no code that plans a request
or lists overrides can see one by construction, rather than by remembering to filter. Consequences:

- Planning, decisions and `explain` cannot show it (it names no target, provider or route).
- `GET /v1/overrides`, its history and `humpyard_overrides_active` do not list or count it, so a client key with
  `read_config` cannot learn admin ids. It has its own gauge, `humpyard_admin_revocations_active`.
- The override removal path only ends an override in force, never a revocation: otherwise a client key that
  manages overrides could end one by id and restore a revoked admin. A test fails if that guard is removed.
- An architecture test allows only the override store, `server/admin.rs` and `server/metrics.rs` to mention
  revocations at all.

Authentication checks the active revocations on each request (the store's active set is already read per
request for planning), so revocation takes effect on the next call with no reload.

*Alternative:* a separate `admin_revocations` table duplicates expiry, audit and removal for no gain. Rejected.

### 5. Audit identity is `admin:<id>`, and revocation names a principal id

Overrides record `created_by` as a plain string that today holds a client key id. An admin id could equal a client
key id, so admin callers are recorded as `admin:<id>` and the two namespaces cannot be confused in the audit
rows. The spec states the creator as that value. The id is an opaque principal string, so a later kind of principal
(for example `oidc:<subject>`) is audited and revoked the same way; only the check that a revocation names a
principal the gateway can authenticate extends, and today that means a configured credential.

### 6. The OpenAPI document is written by hand and tested against the routes

One table in the admin module lists each route, method and required scope. The router is built from it, and a
test compares it with `docs/openapi/admin-v1.json`, which is embedded and served at `/admin/v1/openapi.json`. A
route or scope that changes without the document changing fails the build. This adds no dependency.

*Alternative:* generating the document from handler annotations (`utoipa`) is automatic but adds a dependency
the `deny` job must vet, and the generated schema still needs review. Not needed for about a dozen operations.

### 7. Errors and metrics

Invalid, expired, revoked or wrong-kind credentials answer 401 `admin_credential_invalid` with a
`WWW-Authenticate: Bearer` header and no hint of which; a missing scope answers 403 `admin_scope_required` naming
it. `humpyard_admin_requests_total{result}` is exported only when the admin API is enabled. Admin mutations also
log the credential id.

## Risks / Trade-offs

- **A `credentials` holder can revoke every other admin** → the revocation expires (default four hours, at most
  the overrides maximum), is audited, and the file edit and reload always restores access; the scope is granted
  deliberately.
- **A leaked admin secret lives until revoked or the file changes** → `expires`, revocation without a deploy, and
  separation from client keys; revoke the credential that is compromised.
- **`credentials` lets one admin revoke their own** → allowed; it is how a credential is retired.
- **Two surfaces for config, overrides and reads** → shared cores (decision 3) and tests that call both.
- **Hash-only credentials cannot be recovered** → mint a new one with the existing `keygen` command, whose output
  is a block the operator pastes into the file.
- **The GUI's hosting is undecided, so there is no CORS** → a browser client on another origin cannot call the
  API yet; the follow-up change that picks the hosting adds the narrowest possible origin allowance.

## Open Questions

- Identity-provider sign-in (OIDC) is expected later. It would add a principal kind: the API validates a bearer
  token against the provider's signing keys, with `[admin.oidc]` naming the issuer and audience and mapping
  groups to scopes, and the GUI performs the login itself. It brings a JWT dependency, a runtime fetch of signing
  keys that must fail closed, and the CORS question. This change keeps the seam open and builds none of it.
- ~~Whether `keygen` should print an `[admin.credentials.<id>]` block.~~ Yes: `keygen <id> --admin --scopes ...`, with the block checked by the real configuration loader.
- Whether `ctl` should accept an admin credential and use `/admin/v1`. Deferred: `ctl` works today through the
  client endpoints.
