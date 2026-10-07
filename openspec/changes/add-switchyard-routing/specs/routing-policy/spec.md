# Spec Delta

## Purpose

Defines the extension point that lets budget and provider-health information restrict which
targets a routing algorithm may choose, without changing the algorithms themselves.

## ADDED Requirements

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
