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
The policy SHALL receive the requested route, session id and request metadata, and, when the caller is authenticated, the key id and its budget state, so it can decide per caller.

#### Scenario: Session-aware policy
- **WHEN** a policy decides by session
- **THEN** it can read the session id of the current request

#### Scenario: Budget-aware policy
- **WHEN** an authenticated key is in the restricted state
- **THEN** the policy can read the key id and that state

### Requirement: Degrade instead of fail
When a policy removes every target of a routing tier but other targets remain eligible, the gateway SHALL serve the request from the remaining eligible targets rather than failing.

#### Scenario: Capable tier excluded
- **WHEN** a `stage_router` route would select its capable tier and the policy has excluded every capable target
- **THEN** the request is served by an eligible efficient target and the response headers name the serving target

### Requirement: A cold target is skipped when an alternative exists
When every endpoint of a target is cooling down, the gateway SHALL leave that target out of the route's targets for a request, provided at least one other target remains after classes and the routing policy have narrowed the route. Such a target SHALL NOT receive an attempt from that request.

#### Scenario: Healthy alternative
- **WHEN** a route lists a cold target and a warm target
- **THEN** the request is served by the warm target and the cold target's upstream receives no call

#### Scenario: A cold tier is substituted
- **WHEN** every target of a route's `capable` tier is cold and its `efficient` tier is warm
- **THEN** the efficient targets serve the request, as with any emptied tier

### Requirement: Health never denies service
When every target that classes and the routing policy leave eligible is cold, the gateway SHALL NOT skip any of them: the route SHALL be tried as it would be without health skipping, and the request SHALL NOT be refused because of health.

#### Scenario: Everything is cold
- **WHEN** every eligible target is cooling down
- **THEN** the targets are tried in order and a recovered target serves the request

### Requirement: A cooled target is re-admitted by its probe
A target SHALL stop being cold when its cooldown ends. The next request that reaches it SHALL be admitted as the single probe, and a success SHALL return it to service.

#### Scenario: Recovery
- **WHEN** a cold target's cooldown ends and its upstream is healthy
- **THEN** the next request is sent to it as a probe and, on success, later requests use it normally

### Requirement: Health skipping cannot widen a strict route
Health skipping SHALL apply only within the targets a route's `allowed_classes` and the routing policy leave eligible, and SHALL NOT add a target.

#### Scenario: Strict route with one cold allowed target
- **WHEN** a strict route's only allowed target is cold
- **THEN** it is still tried (health never denies service) and no excluded target is called

### Requirement: Skips are recorded and explained
The decision SHALL list the targets skipped for health, `GET /v1/decisions` SHALL return them, and `explain` SHALL show each target as cold or warm and report when a route would fail open.

#### Scenario: Explain with a cold target
- **WHEN** `explain` describes a request for a route with a cold target and a warm one
- **THEN** the cold target is marked cold and the plan lists only the warm one

### Requirement: Skipping can be turned off
With `[health] skip_cold_targets = false` the gateway SHALL not skip cold targets, and routes SHALL behave as before this change.

#### Scenario: Switch off
- **WHEN** `skip_cold_targets` is false and a route's first target is cold
- **THEN** that target is still tried first

### Requirement: Endpoints may declare what they can take
An endpoint MAY declare `context_window` (tokens), `tools` (boolean) and `vision` (boolean). An endpoint that declares nothing SHALL be treated as capable of any request.

#### Scenario: No declarations
- **WHEN** no endpoint declares a capability
- **THEN** every target is capable and routing is exactly as before

### Requirement: A target that cannot serve a request is removed before routing
For each request, the gateway SHALL remove from the route's targets every target none of whose endpoints can serve it, before the routing algorithm runs. An endpoint cannot serve a request that uses tools if it declares `tools = false`, one that carries images if it declares `vision = false`, or one whose prompt is certainly larger than its declared `context_window`.

#### Scenario: Tools
- **WHEN** a request uses tools, a route lists a target whose only endpoint declares `tools = false` and a target that does not, and the first target's upstream counts calls
- **THEN** the request is served by the second target and the first receives no call

#### Scenario: Context
- **WHEN** a request's prompt is certainly larger than one target's declared window
- **THEN** that target receives no call and a target that can take it serves the request

#### Scenario: Any endpoint is enough
- **WHEN** a target has one endpoint that can serve the request and one that cannot
- **THEN** the target stays in the plan

### Requirement: Removal happens only when it is certain
The gateway SHALL NOT remove a target for context size unless a deliberately low estimate of the prompt exceeds every declared window of the target, so that a target that could have served the request is never removed because of an over-estimate.

#### Scenario: Borderline prompt
- **WHEN** a prompt is close to a target's window but its low estimate is under it
- **THEN** the target stays in the plan and the existing overflow fallback handles a real overflow

### Requirement: A request no target can serve is refused with its reason
When capability removes every target of a route, the gateway SHALL refuse the request before any upstream call with HTTP 400 and outcome `no_capable_target`, naming the reason (tools, images or context).

#### Scenario: Images with no vision target
- **WHEN** a request carries an image and every target of the route declares `vision = false`
- **THEN** the response is 400 `no_capable_target` mentioning images, and no upstream is called

### Requirement: Capability only removes
Capability removal SHALL apply after allowed classes and the routing policy and before health skipping, and SHALL NOT add a target to a plan.

#### Scenario: Strict route
- **WHEN** a strict route's allowed target cannot serve a request
- **THEN** no excluded-class target is added to serve it; the request is refused with `no_capable_target`

### Requirement: Removals are recorded and explained
The decision SHALL list the targets removed for capability, `GET /v1/decisions` SHALL return them, and `explain` SHALL show each target's capability with the reason it would be removed.

#### Scenario: Explain a request with tools
- **WHEN** `explain` describes a request with tools for a route with a `tools = false` target
- **THEN** that target is marked not capable with the reason `tools`

### Requirement: Stated retry times
When an attempt fails with status 429 or 503 and the failure states a retry time, the gateway SHALL cool that endpoint for exactly that time (at least 1 second and at most `[health] max_retry_after_secs`, default 300), opening it immediately instead of after `failure_threshold` failures. The retry time SHALL be read from the upstream response when it is available as a header, and otherwise from a small set of well-known phrasings and fields in the error body, reading at most its first 4 KiB. A failure with no readable retry time SHALL behave as before.

#### Scenario: A 429 with a retry time
- **WHEN** an endpoint returns 429 with a body that says to try again in 20 seconds
- **THEN** the endpoint is skipped for 20 seconds even on its first failure, and one request probes it afterwards

#### Scenario: The cap
- **WHEN** the stated retry time is larger than `max_retry_after_secs`
- **THEN** the endpoint cools for `max_retry_after_secs`

#### Scenario: Unreadable or out of range
- **WHEN** the body has no recognized retry time, or one that is zero or negative or not a number
- **THEN** the failure is handled as before (counted toward `failure_threshold`, guessed cooldown)

#### Scenario: Other statuses
- **WHEN** a 500 response body says to try again in 20 seconds
- **THEN** the stated time is ignored

#### Scenario: Late failures
- **WHEN** a request sent before the endpoint cooled fails while it is cooling
- **THEN** the cooldown is neither shortened nor extended

### Requirement: Cooldown source is visible
Endpoint health SHALL report whether the current cooldown came from a stated retry time or from consecutive failures, and the gateway SHALL count honored retry times by provider and source.

#### Scenario: Health shows the source
- **WHEN** an endpoint is cooling because of a stated retry time
- **THEN** its health entry shows the source `retry_after` and the time remaining

### Requirement: Answers that describe the endpoint count as failures
An upstream answer of 401, 403 or 404, or of 400 or 404 whose error code or type is `model_not_found`, `model_not_available` or `model_decommissioned`, SHALL count as a failure of that endpoint for health, exactly as a 5xx does. A 400 or 422 otherwise SHALL NOT count. A success SHALL reset the count.

#### Scenario: A removed model cools
- **WHEN** an endpoint answers 404 `failure_threshold` times in a row
- **THEN** it is cooling down, is skipped while another target remains, and is probed after the cooldown

#### Scenario: A request error does not count
- **WHEN** an endpoint answers 400 (not a model-gone code) or 422
- **THEN** the endpoint's health is unchanged

#### Scenario: A stray failure does not cool a good endpoint
- **WHEN** an endpoint that usually succeeds answers one 403
- **THEN** it keeps serving

#### Scenario: Health never denies service
- **WHEN** every endpoint of a route is cooling because of these answers
- **THEN** the route is still tried, as with any cooling endpoint

### Requirement: A target whose every endpoint caps below the prompt is removed
An endpoint that declares `max_request_tokens` cannot serve a request whose deliberately low prompt estimate exceeds it. The gateway SHALL remove from a request's plan a target none of whose endpoints can serve it, with the reason `request_cap` when the cap is why, before the routing algorithm runs and before any upstream call.

#### Scenario: An oversized prompt
- **WHEN** a request's low prompt estimate exceeds the `max_request_tokens` of a target's only endpoint, and another target on the route can serve it
- **THEN** the first target receives no call and the other serves the request

#### Scenario: A small prompt
- **WHEN** the same target is asked for a prompt under its cap
- **THEN** it stays in the plan, even if its `context_window` is declared far larger

#### Scenario: Any endpoint is enough
- **WHEN** a target has one endpoint whose cap the prompt exceeds and one that can serve it
- **THEN** the target stays in the plan

### Requirement: A cap removes only when certain, and never adds
The gateway SHALL NOT remove a target for its cap unless the low prompt estimate exceeds the cap of every endpoint of the target, and SHALL NOT use the requested output length in that comparison. A cap SHALL only remove targets from a plan.

#### Scenario: Borderline prompt
- **WHEN** a prompt is close to a target's cap but its low estimate is under it
- **THEN** the target stays in the plan and the existing overflow fallback handles a real refusal

### Requirement: A request every target caps out is refused with its reason
When caps (with any other capability) remove every target of a route, the gateway SHALL refuse the request before any upstream call with HTTP 400 `no_capable_target`, and the message SHALL name the cap as the reason.

#### Scenario: Everything is capped
- **WHEN** a route's only target has an endpoint with `max_request_tokens = 8000` and the prompt is certainly larger
- **THEN** the response is 400 `no_capable_target` mentioning `max_request_tokens`, and no upstream is called

### Requirement: Cap removals are recorded and explained
The decision SHALL list a target removed for its cap with the reason `request_cap`, and `explain` SHALL show it with that reason.

#### Scenario: Explain an oversized prompt
- **WHEN** `explain` describes a prompt larger than a target's cap
- **THEN** the target is marked not capable with the reason `request_cap`
