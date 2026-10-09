# request-routing Specification

## Purpose
Defines how a requested model name becomes a routing decision using Switchyard's built-in
algorithms, and how the decision is reported to the client.

## Requirements

### Requirement: Routes select an algorithm
The gateway SHALL let operators define routes, each with an id clients request as the model and a built-in algorithm of type `passthrough`, `random`, `stage_router` or `llm_classifier` over named targets.

#### Scenario: Stage router route
- **WHEN** a route of type `stage_router` has an efficient and a capable target and a request arrives with failing tool results in its history
- **THEN** the algorithm may select the capable target as its configuration dictates

### Requirement: Targets are directly requestable
The gateway SHALL treat a request naming a target as a passthrough route to that target.

#### Scenario: Direct target
- **WHEN** a client requests model `fast` and `fast` is a target and not a route
- **THEN** the request is served by that target with no routing decision

### Requirement: Ordered fallbacks from the algorithm
The gateway SHALL try the targets an algorithm selects in the order returned, moving to the next target when all endpoints of the current one fail.

#### Scenario: Selected target unavailable
- **WHEN** the algorithm returns targets [capable, efficient] and capable has no working endpoint
- **THEN** the request is served by efficient

### Requirement: Session continuity
The gateway SHALL give algorithms a stable session so latched or counted state persists across requests, using the `x-switchyard-session-id` header when present.

#### Scenario: Escalation latches within a session
- **WHEN** two requests with the same session id arrive and the first caused an escalation
- **THEN** the second is routed with that escalation state

### Requirement: Classifier calls use the provider pool
Routing-time judge calls made by `llm_classifier` routes SHALL be served through the same providers and failover as ordinary requests and SHALL NOT be visible to the client.

#### Scenario: Judge call
- **WHEN** an `llm_classifier` route needs a verdict
- **THEN** the call goes to its configured judge target and only the final answer reaches the client

### Requirement: Serving attribution
The gateway SHALL report the target and provider that served each non-streaming and streaming response in `x-humpyard-target` and `x-humpyard-provider` response headers, and in the request log.

#### Scenario: Header present
- **WHEN** a request is served
- **THEN** both headers name the serving target and provider

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
