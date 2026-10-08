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

### Requirement: Keys, ledger and prices in configuration
The configuration SHALL allow `[keys.<id>]` entries (hash, optional route allowlist, budgets, `over_budget`), a `[ledger]` section naming the SQLite file, a `[budget]` section with the restricted fraction and price ceiling, and an optional `price` (input, output, optional cached input, USD per million tokens) on each endpoint.

#### Scenario: Valid budget config
- **WHEN** the file declares a key with `daily_usd`, a ledger path and priced endpoints
- **THEN** it loads

#### Scenario: Budget without ledger
- **WHEN** a key declares a budget but no ledger path is configured
- **THEN** startup fails explaining that budgets need a ledger

#### Scenario: Invalid limits
- **WHEN** a limit is negative or `restricted_at` is outside 0 to 1
- **THEN** startup fails naming the field

### Requirement: Shutdown grace period
The configuration SHALL accept an optional top-level `shutdown_grace_secs`, a whole number of seconds from 0 to 3600 with a default of 30, bounding how long the gateway waits for in-flight requests after a termination signal.

#### Scenario: Default
- **WHEN** the key is absent
- **THEN** the grace period is 30 seconds

#### Scenario: Out of range
- **WHEN** the value is above 3600
- **THEN** the process exits non-zero naming the key

### Requirement: Health configuration
The config MAY contain a `[health]` section with `failure_threshold` (default 3), `cooldown_secs` (default 30) and `max_cooldown_secs` (default 300). A threshold of 0 SHALL disable endpoint health tracking. Invalid values (cooldown 0 while enabled, max below cooldown, values above one day) SHALL be rejected naming the key.

#### Scenario: Defaults
- **WHEN** the section is absent
- **THEN** health tracking is on with 3, 30 and 300

#### Scenario: Disabled
- **WHEN** `failure_threshold = 0`
- **THEN** no endpoint is ever skipped

#### Scenario: Invalid
- **WHEN** `max_cooldown_secs` is below `cooldown_secs`
- **THEN** loading fails naming `max_cooldown_secs`
