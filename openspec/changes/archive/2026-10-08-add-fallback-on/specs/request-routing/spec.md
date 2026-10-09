# Spec Delta

## ADDED Requirements

### Requirement: Routes choose which failures fall through
A route SHALL accept `fallback_on`, a list of failure classes (`overflow`, `rate_limit`, `timeout`, `server_error`, `connection`, `forbidden`). Only failures in the list SHALL hand the request to the route's next target; the default is all classes. Any other failure SHALL end the request with the original status and message.

#### Scenario: Only overflow falls through
- **WHEN** a route has `fallback_on = ["overflow"]` and its first target answers 503
- **THEN** the client receives the failure and the second target is not called

#### Scenario: Overflow falls through
- **WHEN** the same route's first target reports a context-window overflow
- **THEN** the second target serves the request

#### Scenario: Original error preserved
- **WHEN** a 429 is not in the list
- **THEN** the client receives 429 with the provider's message, in the endpoint's error format

#### Scenario: Fallback disabled
- **WHEN** `fallback_on = []`
- **THEN** no failure moves to the next target

#### Scenario: Unknown class
- **WHEN** the list names a class that does not exist
- **THEN** the config fails to load naming the class
