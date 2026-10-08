# Spec Delta

## ADDED Requirements

### Requirement: Token counting endpoint
The gateway SHALL answer `POST /v1/messages/count_tokens` with an Anthropic-format body `{"input_tokens": n}`, where n is a local estimate of the prompt tokens the requested model would receive, and SHALL mark the response with an `x-humpyard-token-count: estimate` header.

#### Scenario: Count a conversation
- **WHEN** a client posts an Anthropic Messages request body (without `max_tokens`) to `/v1/messages/count_tokens`
- **THEN** the response is 200 with a positive `input_tokens` and the estimate header

#### Scenario: More content counts more
- **WHEN** a request adds more messages or tool definitions
- **THEN** its `input_tokens` is greater than that of the shorter request

### Requirement: Counting makes no upstream call and costs nothing
Token counting SHALL NOT call any provider, SHALL NOT create ledger entries, and SHALL remain available to a key whose budget is exhausted.

#### Scenario: Exhausted key
- **WHEN** a key at its budget limit calls `/v1/messages/count_tokens`
- **THEN** it receives a count rather than a 402, and no ledger entry is written

### Requirement: Counting follows the same access rules as messages
The endpoint SHALL require authentication when keys are configured, SHALL apply the key's route allowlist, and SHALL answer an unknown model with 404, all in Anthropic's error shape.

#### Scenario: Disallowed model
- **WHEN** a key allowed only `auto` asks to count tokens for `smart`
- **THEN** the response is 403 in the Anthropic error format
