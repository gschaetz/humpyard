# Spec Delta

## ADDED Requirements

### Requirement: A rejected request moves a pool on, within a bound
An upstream HTTP 400 or 422 that is not a missing-model answer SHALL have the fallback class `rejected`. The class SHALL be included by default for `fastest` routes and pool routes, SHALL NOT be included by default for any other route, and SHALL be taken exactly as written when a route lists its `fallback_on` classes. A `rejected` answer SHALL move the request to the next endpoint of the same target on every route that allows the class and then to the next target, SHALL NOT count against the endpoint's health, and SHALL NOT be recorded as a speed sample. Once three different providers have rejected the same request, the gateway SHALL stop moving it between targets and return the last rejection to the client with its real status and message (the walk within one target is bounded by that target's endpoints). A guarded tail SHALL be reached by `rejected` only when its `on` list names it.

#### Scenario: A strict member in front of a healthy one
- **WHEN** a pool's first member answers 400 to a request and the second member accepts it
- **THEN** the second member serves the request, the client sees a success, and the first member's health is unchanged

#### Scenario: Every member rejects the request
- **WHEN** three providers in a pool each answer 400
- **THEN** the client receives the last 400 with the provider's message and no further member is tried

#### Scenario: A rejection is not an endpoint failure
- **WHEN** an endpoint answers 400 repeatedly to one client's requests
- **THEN** the endpoint stays in service for other requests and its breaker does not open

#### Scenario: Other routes keep their behavior
- **WHEN** a `passthrough` route without `rejected` in `fallback_on` has a first target that answers 400
- **THEN** the 400 is returned to the client and the second target is not called

#### Scenario: Opting in
- **WHEN** a route lists `fallback_on = ["rejected"]` and its first target answers 422
- **THEN** the request moves to the next target

#### Scenario: The tail is guarded
- **WHEN** every pool member rejects a request and the route's tail lists only `overflow`
- **THEN** the tail is not called

#### Scenario: A model-gone 400 stays unavailable
- **WHEN** an endpoint answers 400 with an error code naming a missing model
- **THEN** it has the class `unavailable` and counts against the endpoint's health, as before
