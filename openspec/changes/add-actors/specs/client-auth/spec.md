# Spec Delta

## ADDED Requirements

### Requirement: Key info shows the key's actor
`GET /v1/key/info` for an authenticated key SHALL also return the id of its actor and of that actor's owner, or null for each that is not set.

#### Scenario: A key with an actor
- **WHEN** a key whose actor is agent `openclaw`, owned by `gary`, calls `GET /v1/key/info`
- **THEN** the answer includes actor `openclaw` and owner `gary`

#### Scenario: A key without one
- **WHEN** a key that names no actor calls `GET /v1/key/info`
- **THEN** actor and owner are null and the rest of the answer is unchanged

### Requirement: A revoked key is refused like an unknown key
While a revocation of a client key is in force, a request with that key SHALL be refused with the same 401 and error as for an unknown or expired key, and SHALL NOT reveal that the key exists or was revoked.

#### Scenario: A revoked key
- **WHEN** a client key is revoked and then used
- **THEN** the answer is identical to the answer for a key the gateway has never known

#### Scenario: The revocation ends
- **WHEN** the revocation expires or is removed and the key is still in the configuration and unexpired
- **THEN** the key authenticates again
