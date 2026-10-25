# Spec Delta

## ADDED Requirements

### Requirement: The configuration may define an admin section
The configuration MAY contain `[admin]` with an optional `listen` address and any number of `[admin.credentials.<id>]` entries (hash, scopes, optional `expires`). An `[admin]` section with no credentials SHALL leave the admin API disabled, and `listen` without credentials SHALL be refused.

#### Scenario: Listen without credentials
- **WHEN** `[admin] listen` is set and no credential is defined
- **THEN** the configuration is refused

#### Scenario: No section
- **WHEN** the configuration has no `[admin]` section
- **THEN** it loads as before and the admin API is disabled

### Requirement: Admin credentials follow the configuration lifecycle
Admin credentials SHALL be part of the configuration snapshot: a reload SHALL apply additions, removals and changed scopes or expiry atomically, a rejected reload SHALL leave the previous credentials in force, and the configuration fingerprint SHALL change when they change.

#### Scenario: Scopes narrowed by reload
- **WHEN** a credential's scopes are reduced in the file and the gateway reloads
- **THEN** the next request is judged against the new scopes

#### Scenario: A bad reload
- **WHEN** a reload is rejected
- **THEN** the credentials from the previous configuration remain in force

### Requirement: Credentials are never echoed to client keys
`GET /v1/config`, which any key with `read_config` may call, SHALL report the admin section only as whether it is enabled, its listener and the number of credentials, never ids, scopes, expiry or hashes. `check-config` and `GET /admin/v1/config` SHALL show each credential by id, scopes and expiry, and none of them SHALL ever show a hash or a secret.

#### Scenario: A client key reads the configuration
- **WHEN** a key with `read_config` reads the effective configuration
- **THEN** it sees the number of admin credentials and nothing that identifies or describes them

#### Scenario: The operator checks a file
- **WHEN** `check-config` runs on a file with admin credentials
- **THEN** it prints each credential's id, scopes and expiry and no hash
