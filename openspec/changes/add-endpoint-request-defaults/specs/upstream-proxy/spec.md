# Spec Delta

## ADDED Requirements

### Requirement: A request carries its endpoint's defaults for the fields it lacks
When the gateway sends a request to an endpoint that declares `request_defaults`, it SHALL add each declared field the request does not already carry and SHALL NOT change a field the request carries. This SHALL hold whether the client's request arrived in the OpenAI or the Anthropic format, and for streaming and non-streaming requests. It SHALL apply to the attempt on that endpoint only: a failover to another endpoint SHALL send that endpoint's defaults, not the first one's.

#### Scenario: The client sent nothing
- **WHEN** an endpoint declares `reasoning_effort = "none"` and a request without `reasoning_effort` is sent to it
- **THEN** the endpoint receives `reasoning_effort: "none"`

#### Scenario: The client's value wins
- **WHEN** the same endpoint receives a request that carries `reasoning_effort: "high"`
- **THEN** the endpoint receives `high`

#### Scenario: An Anthropic-format request
- **WHEN** a request arrives in the Anthropic format without a `temperature` and is sent to an OpenAI-compatible endpoint declaring `temperature = 0.7`
- **THEN** the endpoint receives `temperature: 0.7`

#### Scenario: Failover
- **WHEN** the first endpoint declares `temperature = 0.7`, fails, and the request moves to a second endpoint that declares no defaults
- **THEN** the second endpoint receives the request without a default `temperature`

### Requirement: Defaults do not enter the limits or the cost
A default SHALL NOT change the prompt-size estimate, a budget check, a rate limit or a route choice, because those are decided before an endpoint is chosen. A field that changes how much a response may cost, such as `max_tokens`, SHALL NOT be an allowed default.

#### Scenario: Routing is unchanged
- **WHEN** an endpoint declares defaults
- **THEN** which endpoint a request is sent to, its budget check and its ranking are the same as without them
