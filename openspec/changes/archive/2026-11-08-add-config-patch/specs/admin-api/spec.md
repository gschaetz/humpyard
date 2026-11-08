# Spec Delta

## MODIFIED Requirements

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
