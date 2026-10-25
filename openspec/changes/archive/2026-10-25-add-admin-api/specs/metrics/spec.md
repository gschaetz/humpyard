# Spec Delta

## ADDED Requirements

### Requirement: Admin requests are counted by result
The gateway SHALL count admin API requests in `humpyard_admin_requests_total` with a `result` label (`ok`, `unauthorized`, `forbidden`), the counter SHALL NOT carry the credential id or any secret, and it SHALL be absent when the admin API is disabled.

#### Scenario: A refused credential
- **WHEN** a request to `/admin/v1/whoami` carries an invalid credential
- **THEN** `humpyard_admin_requests_total{result="unauthorized"}` increases by one

#### Scenario: Disabled admin API
- **WHEN** no admin credential is configured
- **THEN** the metric is not exported

### Requirement: Active admin revocations are visible
The gateway SHALL export `humpyard_admin_revocations_active` as the number of active credential revocations, so an alert can fire on one that is unexpected or old.

#### Scenario: A revocation
- **WHEN** a credential is revoked
- **THEN** the gauge is 1 until the revocation ends
