# actors Specification

## Purpose
Names who owns a client key, and which person an agent acts for, so usage can be attributed to both. An actor is a label declared in the configuration file; it grants and removes no authority.

## Requirements

### Requirement: Actors are declared in the configuration
The configuration MAY define `[actors.<id>]` tables. Each SHALL set `kind` to `human` or `agent`, and MAY set `display_name`. An actor id SHALL follow the same rules as a key id. A configuration without `[actors]` SHALL load and behave as before.

#### Scenario: A human and an agent
- **WHEN** the configuration defines `[actors.gary]` with `kind = "human"` and `[actors.claude-code]` with `kind = "agent"`
- **THEN** the configuration loads

#### Scenario: A missing kind
- **WHEN** an actor sets no `kind`
- **THEN** the configuration is rejected naming the actor and `kind`

#### Scenario: An unknown kind
- **WHEN** an actor sets `kind = "bot"`
- **THEN** the configuration is rejected naming the actor and `kind`

#### Scenario: No actors
- **WHEN** the configuration defines no `[actors]`
- **THEN** it loads and every key behaves as it did before

### Requirement: An agent may name the human it acts for
An agent actor MAY set `owner` to the id of a human actor. The configuration SHALL be rejected when `owner` names an actor that does not exist or is not a human, and when a human actor sets `owner`.

#### Scenario: An agent with an owner
- **WHEN** agent `claude-code` sets `owner = "gary"` and `gary` is a human actor
- **THEN** the configuration loads

#### Scenario: An owner that is an agent
- **WHEN** an agent sets `owner` to the id of another agent
- **THEN** the configuration is rejected naming the agent and `owner`

#### Scenario: An owner that does not exist
- **WHEN** an agent sets `owner = "sam"` and no actor `sam` is defined
- **THEN** the configuration is rejected naming the agent and `owner`

#### Scenario: An owner on a human
- **WHEN** a human actor sets `owner`
- **THEN** the configuration is rejected naming the actor and `owner`

### Requirement: A key may name its actor
A key MAY set `actor = "<id>"`. The configuration SHALL be rejected when the id is not a defined actor. A key that names no actor SHALL behave exactly as before.

#### Scenario: A key with an actor
- **WHEN** key `oc-main` sets `actor = "openclaw"` and `openclaw` is defined
- **THEN** the configuration loads and `oc-main` belongs to `openclaw`

#### Scenario: An unknown actor
- **WHEN** a key sets `actor = "openclaw"` and no such actor is defined
- **THEN** the configuration is rejected naming the key and `actor`

#### Scenario: Several keys, one actor
- **WHEN** two keys name `claude-code`
- **THEN** both belong to it

### Requirement: An actor grants and removes nothing
A key's routes, budgets, limits and privileges SHALL come only from the key and its own profile. An actor's `kind`, `owner` and `display_name` SHALL NOT widen or narrow them.

#### Scenario: The same authority with and without an actor
- **WHEN** a key is given an actor and nothing else about it changes
- **THEN** the routes it may use, its budgets and its limits are the same as before

#### Scenario: Kind does not change treatment
- **WHEN** one key belongs to a human actor and another with the same settings to an agent actor
- **THEN** both are treated identically by routing, budgets and limits

### Requirement: The effective configuration shows the actors
The effective configuration SHALL list each actor with its kind, display name, owner and the ids of the keys that name it, labelled `file`, and SHALL show each key's actor. It SHALL NOT show a key's secret or hash.

#### Scenario: Actors in the effective configuration
- **WHEN** an admin reads the effective configuration with an actor and two keys that name it
- **THEN** the actor is listed with both key ids, and no secret or hash appears

### Requirement: A reload applies the actors to the next request
When the configuration is reloaded, a request SHALL use the actors and owners of the new configuration.

#### Scenario: A key moves to another actor
- **WHEN** a key's `actor` changes in the file and the gateway reloads
- **THEN** the key's next request is attributed to the new actor and earlier decisions are unchanged
