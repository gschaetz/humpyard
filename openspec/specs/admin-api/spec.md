# admin-api Specification

## Purpose
An administration surface for the gateway with its own credentials: named per person, scoped, expiring and revocable without a deploy, kept separate from the keys clients use for inference, and described by a published contract.

## Requirements

### Requirement: Admin credentials are named, hashed and scoped
The configuration MAY define `[admin.credentials.<id>]` entries, each with the SHA-256 hash of its secret, a non-empty list of scopes drawn from `read`, `overrides`, `credentials` and `config`, and an optional `expires` time. The gateway SHALL identify an admin caller by that id and SHALL refuse a configuration that contains a plaintext secret, an unknown scope, a duplicate hash, or a hash equal to that of a client key or the metrics credential.

#### Scenario: A valid credential
- **WHEN** a configuration defines `[admin.credentials.gary]` with a hash and `scopes = ["read"]`
- **THEN** it loads and the caller presenting that secret is identified as `gary`

#### Scenario: A secret reused across kinds
- **WHEN** an admin credential's hash equals the hash of a client key
- **THEN** the configuration is refused naming both

#### Scenario: An unknown scope
- **WHEN** a credential lists a scope the gateway does not define
- **THEN** the configuration is refused naming the scope

#### Scenario: The config scope
- **WHEN** a credential lists `config`
- **THEN** it loads, and `humpyard keygen <id> --admin --scopes read,config` accepts the scope and prints a block that loads

### Requirement: Only an admin credential opens the admin API
The gateway SHALL serve the admin API under `/admin/v1`, SHALL open it only to a valid admin credential, and SHALL NOT accept an admin credential on any other path. A client key presented to the admin API SHALL be refused, and an admin credential presented to an inference endpoint SHALL be refused as an invalid key.

#### Scenario: A client key on the admin API
- **WHEN** a valid client key calls `GET /admin/v1/whoami`
- **THEN** the response is 401 with the code `admin_credential_invalid`

#### Scenario: An admin credential on an inference endpoint
- **WHEN** an admin credential calls `POST /v1/chat/completions`
- **THEN** the response is the invalid-key error and no upstream is called

### Requirement: The admin API is absent unless configured, and may have its own listener
Without an `[admin]` section with at least one credential, the gateway SHALL NOT serve `/admin/v1` and SHALL answer it with 404. When `[admin] listen` is set, the admin API SHALL be served only on that address and SHALL NOT be reachable on the main listener.

#### Scenario: No admin section
- **WHEN** no `[admin]` credential is configured
- **THEN** `GET /admin/v1/whoami` is 404

#### Scenario: A separate listener
- **WHEN** `[admin] listen` names an address
- **THEN** `/admin/v1/whoami` is served there and is 404 on the main listener

### Requirement: Credentials expire and are checked on every request
An admin credential past its `expires` time SHALL be refused with 401 and the code `admin_credential_invalid`, and a credential removed by a configuration reload SHALL stop working with that reload.

#### Scenario: An expired credential
- **WHEN** a credential's `expires` has passed
- **THEN** its requests are refused with 401

#### Scenario: Removed by reload
- **WHEN** a credential is deleted from the file and the gateway reloads
- **THEN** its next request is refused

### Requirement: Each endpoint requires its scope
Each admin endpoint SHALL require a stated scope: `read` for reading, `overrides` for creating and removing overrides, `credentials` for revoking a credential, and `config` for producing a configuration patch. A valid credential without the scope SHALL receive 403 with the code `admin_scope_required` naming the scope. The `config` scope SHALL NOT be implied by any other scope, and no other scope SHALL be implied by it.

#### Scenario: A read-only credential
- **WHEN** a credential with only `read` calls `POST /admin/v1/overrides`
- **THEN** the response is 403 `admin_scope_required` naming `overrides` and nothing changes

#### Scenario: Config does not imply read
- **WHEN** a credential with only `config` calls `GET /admin/v1/config`
- **THEN** the response is 403 `admin_scope_required` naming `read`

#### Scenario: Read does not imply config
- **WHEN** a credential with `read`, `overrides` and `credentials` calls `POST /admin/v1/config/patch`
- **THEN** the response is 403 `admin_scope_required` naming `config`

### Requirement: Admin reads cover every key
With the `read` scope, `GET /admin/v1/decisions` and `GET /admin/v1/stats` SHALL cover requests of all keys and SHALL accept an optional `key` filter, `GET /admin/v1/config`, `/health` and `/quota` SHALL return what the matching client endpoints return, and no response SHALL contain a secret, a key hash or a credential hash.

#### Scenario: Decisions across keys
- **WHEN** two keys have made requests and an admin with `read` lists decisions
- **THEN** both keys' decisions are returned, and the `key` filter narrows them to one

#### Scenario: No secrets
- **WHEN** any admin read response is inspected
- **THEN** it contains no API key, key hash or credential hash

### Requirement: Overrides are managed through the admin API with the caller recorded
With the `overrides` scope, an admin SHALL be able to create and remove runtime overrides through `/admin/v1/overrides` under the same validity, expiry and class rules as `/v1/overrides`, and the override and its audit row SHALL record the creator as `admin:<id>`, so an admin id cannot be mistaken for a client key's.

#### Scenario: The creator is the credential
- **WHEN** admin `gary` creates a `disable_target` override
- **THEN** the override and its audit row name `admin:gary` as the creator

#### Scenario: The same rules
- **WHEN** an admin creates an override that names a target the configuration does not have
- **THEN** it is refused exactly as on `/v1/overrides`

### Requirement: A credential can be revoked without a deploy
With the `credentials` scope, an admin SHALL be able to revoke an admin principal by its id through `POST /admin/v1/credentials/{id}/revoke`, with an expiry no later than the overrides maximum. The revocation SHALL be an expiring override that can only remove access, SHALL take effect on the next request without a reload, SHALL be audited with the revoking credential as `admin:<id>`, and SHALL end when it expires or is removed.

#### Scenario: Revoking a colleague
- **WHEN** admin `gary` revokes `sam`
- **THEN** `sam`'s next request is refused with 401, and an audit row names `gary`, `sam` and the expiry

#### Scenario: A revocation ends
- **WHEN** a revocation expires or is removed
- **THEN** the credential works again if it is still in the file and has not expired

#### Scenario: An unknown principal
- **WHEN** a revocation names an id that is not a principal the gateway can authenticate (today, a configured credential)
- **THEN** it is refused and nothing changes

### Requirement: An admin can see who they are and which credentials exist
`GET /admin/v1/whoami` SHALL return the caller's id, scopes and expiry, and `GET /admin/v1/credentials`, with `read`, SHALL list every principal's id, kind, scopes, expiry and whether a revocation is active, and nothing else about it.

#### Scenario: Whoami
- **WHEN** a credential calls `/admin/v1/whoami`
- **THEN** it receives its own id, scopes and expiry

#### Scenario: Listing shows revocation
- **WHEN** a credential is revoked and an admin lists credentials
- **THEN** it appears with its revocation marked and no hash

### Requirement: The admin API publishes its contract
`GET /admin/v1/openapi.json`, with any valid admin credential, SHALL return an OpenAPI document describing every `/admin/v1` operation with its scope, parameters, responses and error codes, and the gateway's tests SHALL fail when the document and the served routes differ.

#### Scenario: Every route is described
- **WHEN** the served admin routes are compared with the document
- **THEN** each route and method appears in the document, and each operation in the document is served

### Requirement: An admin credential can be minted ready to paste
`humpyard keygen <id> --admin --scopes <list> [--expires <datetime>]` SHALL print the new secret once and an `[admin.credentials.<id>]` block holding its hash, scopes and expiry, and the block SHALL have been checked by the configuration loader the gateway uses. It SHALL refuse an unknown scope, an invalid id or expiry, and no scopes, without printing a secret, and `--scopes` and `--expires` SHALL be refused without `--admin`.

#### Scenario: The printed block loads and authenticates
- **WHEN** the block is added to a configuration and the gateway presents the printed secret
- **THEN** the configuration loads and the secret authenticates as that credential with exactly those scopes

#### Scenario: A block the gateway would refuse is never printed
- **WHEN** the scopes include an unknown name
- **THEN** the command fails and prints no secret

### Requirement: Heavy admin reads are limited to one at a time and share results
The admin API SHALL run at most one heavy read at a time (statistics, session views and time-bucketed statistics), SHALL answer another heavy read that arrives meanwhile with 503 `admin_busy` and a `Retry-After` header and SHALL NOT queue it, and SHALL answer an identical read that arrives within ten seconds of an earlier one from that earlier result.

#### Scenario: A second heavy read while one runs
- **WHEN** a statistics read is running and a session view is requested
- **THEN** the second answer is 503 `admin_busy` with `Retry-After`, and the first read is unaffected

#### Scenario: The same read twice
- **WHEN** the same statistics read is made twice within ten seconds
- **THEN** the ledger is read once and both answers are identical

#### Scenario: The same read at the same moment
- **WHEN** five identical statistics reads arrive while the first is still being computed
- **THEN** all five are answered from that one computation, none is answered 503, and the ledger is read once

#### Scenario: A different read is not served another's result
- **WHEN** two statistics reads differ in `window`, `group_by`, a filter or the `key` filter
- **THEN** each is computed for its own parameters

#### Scenario: A read that cannot get a connection
- **WHEN** a heavy or light ledger read cannot get a read connection or lock within the ledger's read limits
- **THEN** the answer is 503 `admin_busy` with `Retry-After`, not a hung request and not a 500

#### Scenario: Light reads are not gated
- **WHEN** a heavy read is running
- **THEN** `GET /admin/v1/health`, `/config`, `/whoami` and the override endpoints are answered normally

### Requirement: Shedding happens after the scope is checked
A heavy admin read SHALL be authenticated and checked for its scope before it can occupy the gate or be answered from a shared result, so a caller without the scope receives the scope error and cannot hold the gate.

#### Scenario: No scope
- **WHEN** a credential without `read` asks for statistics while the gate is free
- **THEN** the answer is 403 `admin_scope_required` and the gate stays free
