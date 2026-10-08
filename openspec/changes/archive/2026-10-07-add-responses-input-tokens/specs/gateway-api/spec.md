# Spec Delta

## ADDED Requirements

### Requirement: Responses input-token counting endpoint
The gateway SHALL answer `POST /v1/responses/input_tokens` with `{"object": "response.input_tokens", "input_tokens": n}`, where n is the same kind of local estimate as the Anthropic counting endpoint, marked with the `x-humpyard-token-count: estimate` header.

#### Scenario: Count a Responses request
- **WHEN** a client posts a Responses request body (`model`, `input`) to `/v1/responses/input_tokens`
- **THEN** the response is 200 with the object `response.input_tokens`, a positive `input_tokens` and the estimate header

#### Scenario: More input counts more
- **WHEN** the request carries more input items, instructions or tool definitions
- **THEN** its `input_tokens` is greater than that of the shorter request

### Requirement: Responses counting shares the counting rules
Responses token counting SHALL make no upstream call, write no ledger entry, remain available to a key whose budget is exhausted, and follow the authentication, allowlist and unknown-model rules of the Responses endpoint, with errors in the Responses error format.

#### Scenario: Unauthenticated
- **WHEN** keys are configured and a request has none
- **THEN** the response is 401 in the OpenAI error format

#### Scenario: Stateful request
- **WHEN** the body carries `previous_response_id`
- **THEN** the response is 400, as for `/v1/responses`
