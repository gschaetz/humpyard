# Spec Delta

## ADDED Requirements

### Requirement: A decision records its actor and owner
A decision SHALL record the id of the actor its key names and the id of that actor's owner, as they were when the request was made. The owner of a human actor SHALL be recorded as that human. A decision whose key names no actor, or an agent with no owner, SHALL record none for the missing id.

#### Scenario: An agent with an owner
- **WHEN** a request is made with a key whose actor is agent `openclaw` owned by `gary`
- **THEN** its decision records actor `openclaw` and owner `gary`

#### Scenario: A key of a human
- **WHEN** a request is made with a key whose actor is human `gary`
- **THEN** its decision records actor `gary` and owner `gary`

#### Scenario: No actor
- **WHEN** a request is made with a key that names no actor, or without a key in open mode
- **THEN** its decision records no actor and no owner

#### Scenario: History does not change
- **WHEN** an actor's owner is changed in the file and the gateway reloads
- **THEN** decisions made before the reload keep the owner they recorded

### Requirement: A decision's actor is not an authority
The actor and owner on a decision SHALL be recorded for attribution only and SHALL NOT be read to authorize, limit or route any request.

#### Scenario: Attribution only
- **WHEN** two requests differ only in the actor their keys name
- **THEN** routing, budgets and limits treat them identically
