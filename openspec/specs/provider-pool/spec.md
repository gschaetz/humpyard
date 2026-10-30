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

### Requirement: Endpoints that keep failing are skipped for a cooldown
After `failure_threshold` consecutive failover-class failures an endpoint SHALL be skipped for `cooldown_secs`, with the cooldown doubling on each repeated trip up to `max_cooldown_secs`. A success SHALL reset the endpoint to healthy.

#### Scenario: Breaker opens
- **WHEN** the first endpoint of a target fails `failure_threshold` times in a row
- **THEN** the next request goes straight to the next endpoint without contacting the failed one

#### Scenario: Recovery probe
- **WHEN** the cooldown has passed
- **THEN** exactly one request is sent to the endpoint as a probe; success restores it and failure reopens it with a longer cooldown

#### Scenario: Non-failover errors do not count
- **WHEN** an endpoint answers with a 4xx other than 408 and 429
- **THEN** its failure count is unchanged

### Requirement: Health never denies service
If every endpoint of a target is cooling down, the gateway SHALL still try them, those that reopen soonest first.

#### Scenario: All cold
- **WHEN** all endpoints of a target are open
- **THEN** the request is attempted against them rather than failing immediately

### Requirement: Health is observable
The gateway SHALL log each endpoint's transition between healthy and cooling down, and `GET /v1/health` SHALL report each endpoint's state, consecutive failures and remaining cooldown.

#### Scenario: Health report
- **WHEN** an authorized client calls `GET /v1/health`
- **THEN** it receives every configured endpoint with provider, model and state

### Requirement: Cross-target fallback on timeout
When all endpoints of a target fail and the last failure was a timeout, the gateway SHALL try the route's next target, and SHALL return the usual 504 timeout error if none succeeds.

#### Scenario: Timeout then fallback
- **WHEN** the first target times out and the second answers
- **THEN** the request is served by the second target

#### Scenario: Nothing to fall back to
- **WHEN** the only target times out
- **THEN** the client receives 504 with the message `upstream timed out`

### Requirement: Fallback matrix is pinned
Context-window overflow, 429, 403 and 5xx SHALL fall back to the next target; other client errors (400, 401, 404) SHALL stop at once and reach the client.

#### Scenario: Client error does not fall back
- **WHEN** the first target answers 404
- **THEN** the client receives 404 and the second target is not contacted

### Requirement: A provider's timeout bounds the wait for a response to start
A provider's `timeout_secs` SHALL bound the time the gateway waits for that provider to begin a response, counted from the start of the call and including the provider's retries. When it passes before the response has begun, the call SHALL fail with a timeout, which fails over to the next endpoint or target exactly as any other timeout does.

#### Scenario: A provider that never starts
- **WHEN** a provider sends nothing for longer than its `timeout_secs`
- **THEN** the call fails with a timeout and the request moves to the next endpoint or target

#### Scenario: A provider that answers in time
- **WHEN** a provider begins its response within `timeout_secs`
- **THEN** the wait ends and no timeout is raised for the time that follows

### Requirement: A started stream is limited by idle time, not by its length
Once a streamed answer has begun, `timeout_secs` SHALL limit how long the stream may send nothing, counted from the last chunk received. A stream that sends nothing for `timeout_secs` SHALL end with the timeout error. A stream that keeps sending chunks SHALL NOT be ended because of its total length, however long it runs.

#### Scenario: A long stream that keeps producing
- **WHEN** a streamed answer runs for longer than `timeout_secs` in total and a chunk arrives within every `timeout_secs`
- **THEN** it completes and the client receives the whole answer

#### Scenario: A stream that stalls
- **WHEN** a started stream sends nothing for `timeout_secs`
- **THEN** it ends with the timeout error, and the failure is recorded as a timeout

#### Scenario: A slow start followed by a healthy stream
- **WHEN** a provider takes most of `timeout_secs` to begin a stream and then sends chunks steadily
- **THEN** the stream is not ended when the original wait would have run out

### Requirement: A buffered answer is bounded as a whole
A response that is not streamed has no earlier point at which it starts, so `timeout_secs` SHALL bound the whole call, retries included, for it.

#### Scenario: A buffered answer that takes too long
- **WHEN** a non-streamed call does not finish within `timeout_secs`
- **THEN** it fails with a timeout and fails over as before

### Requirement: A timeout says which limit was reached
The timeout error SHALL say whether the wait for the response to start, the idle time of a stream, or the whole of a buffered call ran out, with the configured limit, so a log line or a recorded attempt can be read without the configuration at hand. The failure class SHALL stay `timeout` in every case.

#### Scenario: A stalled stream is logged with its reason
- **WHEN** a started stream is ended for being idle
- **THEN** the failure message names the idle limit and the failure class is `timeout`
