# Proposal

## Why

An operator surface is planned (ADR 0025, `docs/architecture.md` "Direction: operating humpyard"), and its first
step is an admin API with its own credential. Today there is no such thing. Operating the gateway means using an
ordinary client key that carries two flags (`read_config`, `manage_overrides`), which has three limits that a GUI
or a second administrator would hit at once:

- **No identity to audit.** One key stands for "the operator", so an override's `created_by` cannot tell two
  people apart, and one person's access cannot be removed without rotating the secret everyone shares.
- **No view across keys.** Decisions and statistics are scoped to the calling key by design, so the operator's own
  key, which spends nothing, sees an empty gateway.
- **No separation from clients.** Authority to operate rides on the same kind of credential that clients hold, so
  the planned database-managed keys (ADR 0025) could not be kept from minting more keys.

More than one person will administer this gateway, so the credential has to be per person, scoped, expiring and
revocable without a deploy. A GUI is then just another client of this API.

## What Changes

- A new `[admin]` configuration section with named credentials, `[admin.credentials.<id>]`: a hash, scopes
  (`read`, `overrides`, `credentials`) and an optional expiry. An optional `[admin] listen` serves the admin API
  on its own address, as `[metrics] listen` does.
- A versioned namespace, `/admin/v1/...`, that only an admin credential opens. A client key never opens it, and
  an admin credential never opens an inference endpoint or any other path.
- Read endpoints across **all** keys: the effective configuration, health, decisions, statistics and quota,
  with an optional `key` filter. The existing per-key endpoints are unchanged.
- Override management through the same store and rules as today, with the credential's id recorded as the
  creator.
- Revoking an admin credential without a deploy: an expiring override of a new kind that only removes access,
  and an immediate effect on the next request. A credential can also be removed by editing the file and reloading.
- `GET /admin/v1/credentials` and `GET /admin/v1/whoami`: who the caller is, their scopes and expiry, and which
  credentials exist, never a secret or a hash.
- An OpenAPI document for `/admin/v1`, served by the gateway and kept in step with the routes by a test.
- A counter for admin requests by result, and a refusal reason for rejected ones.
- **Not changed:** `read_config`, `manage_overrides`, `/v1/config` and `/v1/overrides` keep working for `ctl` and
  existing clients.
- **Out of scope:** the GUI itself, CORS (the GUI's hosting is undecided), `ctl` support for admin credentials,
  key management (needs profiles and database keys, tasks 5.3 and 5.4), patch generation and direct config writes
  (5.1 and 5.2), and per-profile rate limits (5.5, which only matters once client keys are shared).

## Capabilities

### New Capabilities
- `admin-api`: the admin credential model (named, scoped, expiring, revocable), the `/admin/v1` namespace and its
  separation from client keys, the all-keys read endpoints, override management and credential revocation through
  it, and the OpenAPI contract.

### Modified Capabilities
- `gateway-config`: the `[admin]` section, its validation and its reload behavior.
- `client-auth`: a client key never opens the admin API, and an admin credential never serves inference.
- `metrics`: the admin request counter and the new refusal reason.

## Impact

Configuration (`[admin]`), a new `src/server/admin` module and a credential store in the planning snapshot, the
override store (one new kind, and `created_by` accepting a credential id), `GET /admin/v1/openapi.json`, one
counter, and docs (`api.md`, `configuration.md`, `metrics.md`, `architecture.md`, `roadmap.md`) plus a new ADR.
No change for a gateway without an `[admin]` section: the namespace is simply absent. No new dependency is
planned; the OpenAPI document is written by hand and checked by a test.
