# gateway-config Specification

## Purpose
Defines how operators configure and start the gateway, and how credentials are supplied.

## Requirements

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

### Requirement: Config validation command
The CLI SHALL provide `check-config <path>` that validates a file without starting the server.

#### Scenario: Check a good file
- **WHEN** `check-config` runs on a valid file
- **THEN** it exits 0

### Requirement: Provider headers
The configuration SHALL allow each provider an optional `headers` table of additional HTTP headers, and SHALL reject header names or values that are not valid HTTP and names that would override authentication.

#### Scenario: Valid headers
- **WHEN** a provider declares `headers = { "x-app" = "conductor" }`
- **THEN** the configuration loads

#### Scenario: Authentication header rejected
- **WHEN** a provider declares a header named `authorization` or `x-api-key`
- **THEN** the process exits non-zero naming the provider and the header

#### Scenario: Invalid header name
- **WHEN** a provider declares a header whose name contains whitespace
- **THEN** the process exits non-zero naming the provider and the header
