# Spec Delta

## Purpose

Defines how clients identify themselves to the gateway with virtual keys, and what an
authenticated key may do.

## ADDED Requirements

### Requirement: Virtual keys defined by hash
The gateway SHALL accept keys defined in configuration by a stable id and the SHA-256 hash of the key, and SHALL reject configuration that contains a plaintext key.

#### Scenario: Valid key definition
- **WHEN** a key is declared with an id and a `sha256:` hash
- **THEN** the configuration loads and the key can authenticate

#### Scenario: Plaintext key rejected
- **WHEN** a key definition contains a field holding a plaintext key
- **THEN** startup fails naming the key id

### Requirement: Authentication when keys exist
When at least one key is configured, the gateway SHALL require a valid key on every inference endpoint, accepted as `Authorization: Bearer <key>` or `x-api-key: <key>`, and SHALL answer an invalid or missing key with HTTP 401 in the endpoint's error format.

#### Scenario: Missing key
- **WHEN** a request without a key reaches `/v1/messages` and keys are configured
- **THEN** the response is 401 in the Anthropic error format

#### Scenario: Anthropic-style header
- **WHEN** a client sends the key in `x-api-key`
- **THEN** it authenticates as with a bearer token

### Requirement: Open mode without keys
When no keys are configured the gateway SHALL accept requests without a key and SHALL NOT apply budgets.

#### Scenario: No keys configured
- **WHEN** the configuration declares no keys
- **THEN** requests without credentials are served exactly as before

### Requirement: Route allowlist
A key MAY list the routes and targets it may request, and the gateway SHALL answer a request for any other model with HTTP 403 in the endpoint's error format.

#### Scenario: Disallowed route
- **WHEN** a key allowed only `auto` requests `smart`
- **THEN** the response is 403 and no upstream call is made

### Requirement: Key info endpoint
The gateway SHALL answer `GET /v1/key/info` for an authenticated key with its id, its limits, its current spend per period and its budget state.

#### Scenario: Key reads its own state
- **WHEN** a key with a $5 daily limit that has spent $1 calls `/v1/key/info`
- **THEN** the response shows the $5 limit, $1 spent, and the state `healthy`

### Requirement: Key generation command
The CLI SHALL provide `keygen <id>` that prints a new random key once and the configuration block containing its hash.

#### Scenario: Generate a key
- **WHEN** `keygen alice` runs
- **THEN** it prints a key and a `[keys.alice]` block whose hash matches that key, and stores nothing

### Requirement: Keys never logged
The gateway SHALL NOT write a key or its hash to logs or the ledger; it SHALL identify callers by key id.

#### Scenario: Request log
- **WHEN** an authenticated request is logged
- **THEN** the log line names the key id and contains neither the key nor its hash
