# Spec Delta

## ADDED Requirements

### Requirement: A provider may have request fields removed
A provider MAY set `drop_fields`, a list of top-level request field names, and the gateway SHALL remove those fields from every request it sends to that provider's endpoints, including from a preserved same-format request body. A provider that sets nothing SHALL receive the request unchanged by this rule, and a field not present SHALL be ignored.

#### Scenario: A strict provider is sent no cache key
- **WHEN** a Responses request carrying `prompt_cache_key` and `store` is routed to a provider whose `drop_fields` lists both
- **THEN** the upstream body has neither field and every other field is unchanged

#### Scenario: Other providers still get the cache key
- **WHEN** the same request is routed to a provider with no `drop_fields`
- **THEN** the upstream body still carries `prompt_cache_key`

#### Scenario: A chat client is covered too
- **WHEN** a chat-completions request carrying a listed field is routed to that provider
- **THEN** the upstream body does not carry it

#### Scenario: Failover applies each provider's own list
- **WHEN** a request fails over from a provider with `drop_fields` to one without
- **THEN** the second provider receives the field

### Requirement: An error array carries the provider's message
When an upstream error body is a JSON array whose first element has the `error.message` shape, the gateway SHALL use that message as it does for a bare object.

#### Scenario: Google-style error array
- **WHEN** the upstream body is `[{"error": {"message": "Unknown name \"store\""}}]`
- **THEN** the client's error message is `Unknown name "store"`
