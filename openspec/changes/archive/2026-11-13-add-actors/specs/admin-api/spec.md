# Spec Delta

## ADDED Requirements

### Requirement: A client key can be revoked without a deploy
With the `credentials` scope, an admin SHALL be able to revoke a client key by its id through `POST /admin/v1/keys/{id}/revoke`, with an expiry no later than the overrides maximum, and to remove the revocation through `DELETE` on the same path. The revocation SHALL only remove access, SHALL take effect on the next request without a reload, SHALL be audited with the revoking credential as `admin:<id>`, and SHALL end when it expires or is removed.

#### Scenario: Revoking a leaked key
- **WHEN** admin `gary` revokes key `oc-main`
- **THEN** the key's next request is refused with 401, and an audit row names `gary`, `oc-main` and the expiry

#### Scenario: A revocation ends
- **WHEN** a revocation expires or is removed
- **THEN** the key works again if it is still in the file and has not expired

#### Scenario: An unknown key
- **WHEN** a revocation names an id that is not a client key in the configuration
- **THEN** it is refused and nothing changes

#### Scenario: The wrong scope
- **WHEN** a credential with `overrides` but not `credentials` calls the endpoint
- **THEN** it is refused with 403

#### Scenario: A client key cannot do it
- **WHEN** a client key calls the endpoint
- **THEN** it is answered with the same 401 as for any admin endpoint

### Requirement: A key revocation is not an override
A client-key revocation SHALL NOT be listed by the override endpoints, counted in `humpyard_overrides_active`, or ended through them, and SHALL NOT affect routing.

#### Scenario: Not listed as an override
- **WHEN** a key is revoked and an admin lists overrides
- **THEN** the revocation is not among them

#### Scenario: Not ended through the override endpoints
- **WHEN** a credential with `overrides` tries to delete the revocation as an override
- **THEN** nothing changes

### Requirement: The effective configuration shows a revoked key
`GET /admin/v1/config` SHALL mark a client key whose revocation is in force as `revoked: true`, with the revocation's expiry, and SHALL NOT show its secret or hash.

#### Scenario: A revoked key is marked
- **WHEN** a key is revoked and an admin reads the effective configuration
- **THEN** the key appears with `revoked: true` and its expiry, and no hash
