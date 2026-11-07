# Spec Delta

## ADDED Requirements

### Requirement: Model aliases
While `model_aliases` is on, the gateway SHALL accept a configured alias as the `model` of any inference or token-counting request and treat the request as if it named the alias's target. Every later stage, including key authorization, selector rules, the decision log and the ledger, SHALL see the target name, never the alias. Requests that name a route or target directly SHALL behave as before.

#### Scenario: Alias resolves to its route
- **WHEN** alias `claude-deep` points at route `deep-thinker-public` and a client posts `/v1/messages` with `"model": "claude-deep"`
- **THEN** the request is served by `deep-thinker-public` and the decision log and ledger record `deep-thinker-public`

#### Scenario: Real names keep working
- **WHEN** a client posts a request naming `deep-thinker-public` directly
- **THEN** the request is served exactly as before the alias existed

#### Scenario: Unknown name
- **WHEN** a client names a model that is neither a route, a target nor an alias
- **THEN** the gateway returns the existing model-not-found error

### Requirement: Aliases follow the key's allowlist
A key SHALL be allowed to use an alias exactly when it is allowed to use the alias's target. A key's `allowed_routes` SHALL list real route or target names only.

#### Scenario: Allowed through the target
- **WHEN** a key's `allowed_routes` lists `deep-thinker-public` and it requests alias `claude-deep` for that route
- **THEN** the request is authorized

#### Scenario: Refused through the target
- **WHEN** a key's `allowed_routes` does not list `coding-paid` and it requests alias `claude-coding` for that route
- **THEN** the gateway refuses with the same forbidden error as for the real name, naming the real name

## MODIFIED Requirements

### Requirement: Model listing
The gateway SHALL answer `GET /v1/models` with an OpenAI-format list containing every configured route and target name and every configured alias the caller's key may use. An alias entry SHALL carry `display_name` and `description` when configured; entries for real names SHALL be unchanged.

#### Scenario: List models
- **WHEN** a client requests `/v1/models`
- **THEN** the response lists each route and target name from the configuration

#### Scenario: Aliases are listed with their labels
- **WHEN** alias `claude-deep` is configured with a display name and description and the caller's key may use its target
- **THEN** the response includes an entry with id `claude-deep` carrying that `display_name` and `description`

#### Scenario: Aliases the key may not use are hidden
- **WHEN** an alias points at a route the caller's key may not use
- **THEN** the alias is absent from the response, as the real name is
