# Spec Delta

## MODIFIED Requirements

### Requirement: TOML configuration
The gateway SHALL load a TOML file defining the listen address, any number of providers (name, base URL, API key env var name, timeout), targets with ordered endpoints, and routes with a built-in algorithm over named targets.

#### Scenario: Valid config
- **WHEN** `serve --config <path>` is run with a valid file
- **THEN** the server listens on the configured address

#### Scenario: Invalid config
- **WHEN** the file is missing a required field or has an unknown key
- **THEN** the process exits non-zero with a message naming the problem

#### Scenario: Dangling reference
- **WHEN** an endpoint names an unknown provider, or a route names an unknown target
- **THEN** the process exits non-zero naming the missing reference

#### Scenario: Duplicate names
- **WHEN** two providers, targets or routes share a name, or a route and a target share a name
- **THEN** the process exits non-zero naming the duplicate

### Requirement: API keys come from the environment
The configuration SHALL name an environment variable for each provider's API key and SHALL reject an inline key.

#### Scenario: Missing key variable
- **WHEN** a provider's named environment variable is unset at startup
- **THEN** startup fails with an error naming the provider and the variable
