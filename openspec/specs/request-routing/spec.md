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

### Requirement: 410 and 402 describe the endpoint
An upstream answer of HTTP 410 (Gone) or 402 (Payment Required) SHALL be treated as describing the endpoint, not the request: it SHALL count against the endpoint's health like a 5xx, move to the next endpoint of the same target, carry the `unavailable` fallback class, and be restored to its real status if no target answers.

#### Scenario: A pool member answers 410
- **WHEN** the first member of a `fastest` pool route answers 410 and the second answers 200
- **THEN** the request is served by the second member, and the first counts a failure toward its cooldown

#### Scenario: Every member is gone
- **WHEN** every target answers 410
- **THEN** the client receives the real 410, not a 503

#### Scenario: Payment required
- **WHEN** a pool member answers 402 and another member answers 200
- **THEN** the request is served by the other member and the 402 member cools after the failure threshold

### Requirement: 413 is overflow and does not count against health
An upstream answer of HTTP 413 SHALL carry the `overflow` fallback class, SHALL move to the next endpoint of the same target and, where the route's guard allows `overflow`, to the next target, SHALL NOT count against the endpoint's health, and SHALL be restored to its real status and message if no target answers.

#### Scenario: A pool member is too small for the request
- **WHEN** the first member of a `fastest` pool route answers 413 and the second answers 200
- **THEN** the request is served by the second member

#### Scenario: A 413 does not cool the endpoint
- **WHEN** an endpoint answers 413 on more consecutive requests than the failure threshold
- **THEN** it is still asked on the next request and is not cooling down

#### Scenario: Every member is too small
- **WHEN** every pool member answers 413 and the route has a paid tail whose guard lists `overflow`
- **THEN** the request is served by the tail

#### Scenario: Nothing takes the request
- **WHEN** every target answers 413
- **THEN** the client receives 413 with the provider's message

### Requirement: Active overrides only remove candidates
Active overrides SHALL be applied in planning after class strictness and capabilities and before health, and SHALL only remove candidates: `disable_target` removes a target, `drain_provider` removes every endpoint of a provider, and `pin_route` cuts a route's candidates to the pinned target. An override SHALL stop having any effect at its expiry without any other action.

#### Scenario: A disabled target is skipped
- **WHEN** a target is disabled and a route would otherwise try it
- **THEN** the request is served by another candidate and the disabled target is not called

#### Scenario: An override expires
- **WHEN** a target is disabled for one hour and an hour passes
- **THEN** the target is eligible again with no other action

#### Scenario: A drained provider
- **WHEN** a provider is drained
- **THEN** none of its endpoints is called, and a target with endpoints on other providers still serves through those

#### Scenario: A pin
- **WHEN** a route is pinned to one of its targets
- **THEN** only that target serves the route until the pin ends

### Requirement: An override that removes every candidate refuses the request
When overrides remove every candidate, the gateway SHALL refuse the request with HTTP 503 and the error code `override_no_target`, before any upstream call, and the decision SHALL record the overrides that applied.

#### Scenario: The last target is disabled
- **WHEN** the only eligible target of a route is disabled
- **THEN** the request is refused with `override_no_target` and removing the override restores service at once
