# routing-policy Specification

## Purpose
Defines the extension point that lets budget and provider-health information restrict which
targets a routing algorithm may choose, without changing the algorithms themselves.

## Requirements

### Requirement: Eligibility policy per request
Before routing each request the gateway SHALL ask a routing policy which targets are eligible, and algorithms SHALL only choose among eligible targets.

#### Scenario: Default policy
- **WHEN** no policy is configured
- **THEN** every configured target is eligible

#### Scenario: Policy removes a target
- **WHEN** a policy marks target `capable` ineligible
- **THEN** no algorithm selects `capable` for that request, including as a fallback

### Requirement: No eligible targets
The gateway SHALL return HTTP 503 in the endpoint's error format when the policy leaves a route with no eligible target.

#### Scenario: Everything excluded
- **WHEN** the policy excludes all targets of the requested route
- **THEN** the client receives 503 naming the route and the reason

### Requirement: Policy context
The policy SHALL receive the requested route, session id and request metadata so it can decide per caller.

#### Scenario: Session-aware policy
- **WHEN** a policy decides by session
- **THEN** it can read the session id of the current request

### Requirement: Degrade instead of fail
When a policy removes every target of a routing tier but other targets remain eligible, the gateway SHALL serve the request from the remaining eligible targets rather than failing.

#### Scenario: Capable tier excluded
- **WHEN** a `stage_router` route would select its capable tier and the policy has excluded every capable target
- **THEN** the request is served by an eligible efficient target and the response headers name the serving target
