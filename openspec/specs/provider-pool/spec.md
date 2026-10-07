# provider-pool Specification

## Purpose
Defines how upstream providers and the models they serve are described, and how the gateway
fails over between providers that serve the same target.

## Requirements

### Requirement: Providers and targets
The gateway SHALL support any number of OpenAI-compatible providers and any number of targets, where a target is a named model served by an ordered list of provider endpoints.

#### Scenario: Target with two endpoints
- **WHEN** a target lists endpoint A then endpoint B
- **THEN** requests for that target try A first and B only if A fails over

### Requirement: Failover between endpoints
The gateway SHALL try the next endpoint of a target when the current one fails with a connection error, timeout, HTTP 5xx or HTTP 429, before any response bytes have reached the client.

#### Scenario: First provider rate limited
- **WHEN** endpoint A returns 429 and endpoint B succeeds
- **THEN** the client receives B's response

#### Scenario: All endpoints fail
- **WHEN** every endpoint of the target fails
- **THEN** the client receives the error of the last endpoint tried, mapped per upstream error rules

#### Scenario: Failure after streaming began
- **WHEN** an endpoint fails after the first stream chunk was sent to the client
- **THEN** the gateway ends the stream with an error event and does not fail over

### Requirement: Client errors do not fail over
The gateway SHALL NOT fail over on HTTP 4xx other than 429, because another endpoint would reject the same request.

#### Scenario: Bad request
- **WHEN** an endpoint returns 400
- **THEN** the client receives that 400 immediately

### Requirement: Per-endpoint model name
Each endpoint SHALL carry the model name sent to its provider, which may differ from the target name.

#### Scenario: Different upstream names
- **WHEN** target `fast` has endpoint A with model `llama-3.3-70b` and endpoint B with model `llama3.3`
- **THEN** each provider receives its own model name

### Requirement: Configured provider headers are sent
The gateway SHALL send each provider's configured headers on every upstream call to that provider, and only to that provider.

#### Scenario: Header on every call
- **WHEN** a provider configures `x-app = "conductor"` and a request is routed to it
- **THEN** the upstream request carries `x-app: conductor`

#### Scenario: Not sent to other providers
- **WHEN** a request fails over from provider A (with headers) to provider B (without)
- **THEN** provider B's request does not carry A's headers
