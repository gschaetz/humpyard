## ADDED Requirements

### Requirement: Time-bucketed statistics are an admin read
`GET /admin/v1/stats/series` SHALL require the `read` scope, SHALL cover requests of all keys with an optional `key` filter, SHALL accept `window`, `interval`, `group_by` (route, target or endpoint), `route`, `target` and `limit`, SHALL reject any other parameter and any value outside the documented bounds with a 400 that names the parameter, and SHALL NOT be offered on the client API.

#### Scenario: Scope is checked before anything is read
- **WHEN** a credential without `read` calls `GET /admin/v1/stats/series`
- **THEN** the response is 403 `admin_scope_required` naming `read`

#### Scenario: A client key cannot read it
- **WHEN** a client key calls `GET /v1/stats/series`
- **THEN** the response is 404, as for any path the client API does not have

#### Scenario: An unknown parameter
- **WHEN** a series is requested with `bucket=5m`
- **THEN** the response is a 400 saying `bucket` is not a parameter, and nothing is read

#### Scenario: Across keys
- **WHEN** two keys have made requests and an admin with `read` reads a series
- **THEN** both keys' requests are counted, and the `key` filter narrows the series to one

### Requirement: The series response contains no secret and is in the published contract
The series response SHALL contain only group names, counts, rates, latencies and times, never a secret, a key hash, a credential hash or request content, and the endpoint, its parameters and its response SHALL be described in `docs/openapi/admin-v1.json` and covered by the contract test.

#### Scenario: The contract names every field
- **WHEN** the contract test compares the route table with the published document
- **THEN** `/admin/v1/stats/series` and each field of a bucket are described, and the test fails if the route or a field is missing
