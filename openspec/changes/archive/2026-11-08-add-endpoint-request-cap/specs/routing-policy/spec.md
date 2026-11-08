# Spec Delta

## ADDED Requirements

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
