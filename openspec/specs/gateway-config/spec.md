# gateway-config Specification

## Purpose
Defines how operators configure and start the gateway, and how credentials are supplied.

## Requirements

### Requirement: TOML configuration
The gateway SHALL load a TOML file defining the listen address, one upstream (base URL, API key env var name, timeout), and the model names it serves.

#### Scenario: Valid config
- **WHEN** `serve --config <path>` is run with a valid file
- **THEN** the server listens on the configured address

#### Scenario: Invalid config
- **WHEN** the file is missing a required field or has an unknown key
- **THEN** the process exits non-zero with a message naming the problem

### Requirement: API keys come from the environment
The configuration SHALL name an environment variable for the upstream API key and SHALL reject an inline key.

#### Scenario: Missing key variable
- **WHEN** the named environment variable is unset at startup
- **THEN** startup fails with an error naming the variable

### Requirement: Config validation command
The CLI SHALL provide `check-config <path>` that validates a file without starting the server.

#### Scenario: Check a good file
- **WHEN** `check-config` runs on a valid file
- **THEN** it exits 0
