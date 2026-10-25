# Spec Delta

## ADDED Requirements

### Requirement: Client keys and admin credentials never substitute for each other
A client key SHALL NOT open the admin API, whatever flags it carries, and an admin credential SHALL NOT authenticate a request to any inference or client endpoint. The existing `read_config` and `manage_overrides` flags SHALL keep their present meaning on `/v1/config` and `/v1/overrides` and SHALL NOT grant access to `/admin/v1`.

#### Scenario: A flagged client key
- **WHEN** a client key with `read_config = true` and `manage_overrides = true` calls `/admin/v1/config`
- **THEN** it is refused

#### Scenario: Existing endpoints unchanged
- **WHEN** a client key with `manage_overrides = true` calls `POST /v1/overrides`
- **THEN** it behaves exactly as before this change
