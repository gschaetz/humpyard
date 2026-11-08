# Spec Delta

## ADDED Requirements

### Requirement: The effective configuration shows profiles
`GET /v1/config` SHALL show, for each key, the name of its profile (`null` when none) and its resolved settings, and SHALL list the defined profiles with their settings. It SHALL NOT show a key hash.

#### Scenario: A key with a profile
- **WHEN** a key names a profile and a client with `read_config` reads `GET /v1/config`
- **THEN** the key's entry names the profile and shows the values it ended up with, inherited or its own

#### Scenario: The profiles are listed
- **WHEN** two profiles are defined
- **THEN** the response lists both with their settings
