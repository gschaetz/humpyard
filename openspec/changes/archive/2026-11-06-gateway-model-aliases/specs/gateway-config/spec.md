# Spec Delta

## ADDED Requirements

### Requirement: Model aliases
The configuration MAY define `[aliases.<id>]` tables, each with a required `model` naming an existing route or target, and optional `display_name` and `description` strings. A configuration with no `[aliases]` SHALL load and behave as before.

#### Scenario: Valid alias
- **WHEN** `[aliases.claude-deep]` has `model = "deep-thinker-public"` and that route exists
- **THEN** the configuration loads and `claude-deep` is a requestable model

#### Scenario: Dangling alias
- **WHEN** an alias's `model` names no route or target
- **THEN** the configuration is rejected with a message naming the alias and listing the names that exist

#### Scenario: A key cannot grant an alias
- **WHEN** a key's `allowed_routes` names an alias
- **THEN** the configuration is rejected with the existing unknown route or target message

#### Scenario: Reload applies aliases
- **WHEN** a reload adds, changes or removes an alias
- **THEN** new requests use the new alias set and requests already in flight are unaffected

### Requirement: Alias names and labels are validated
An alias id SHALL be 1 to 64 characters of lowercase letters, digits, `.`, `_` and `-`, and SHALL NOT equal any route, target or other alias name. A `display_name` SHALL be at most 64 characters and a `description` at most 256, with no control characters.

#### Scenario: Collision
- **WHEN** an alias id equals a route, target or another alias name
- **THEN** the configuration is rejected with a message naming the alias and the name it collides with

#### Scenario: Invalid fields
- **WHEN** an alias id has an uppercase letter or a space, or a label is over its limit or holds a control character
- **THEN** the configuration is rejected with a message naming the alias and the field

### Requirement: Aliases can be switched off
The configuration MAY set the top-level `model_aliases` to `true` or `false`; it SHALL default to `true`. When `false`, the gateway SHALL NOT resolve aliases or list them, while the `[aliases]` tables are still parsed and validated.

#### Scenario: Default on
- **WHEN** `model_aliases` is absent and aliases are configured
- **THEN** aliases are resolved and listed

#### Scenario: Switched off
- **WHEN** `model_aliases = false` and a client requests an alias
- **THEN** the gateway returns the model-not-found error and `/v1/models` lists no alias

#### Scenario: Invalid aliases still rejected when off
- **WHEN** `model_aliases = false` and an alias collides with a route name
- **THEN** the configuration is rejected, so turning the switch on later cannot fail a reload
