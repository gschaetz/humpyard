# Spec Delta

## ADDED Requirements

### Requirement: Replayed tool calls carry a provider's configured extra content
A provider MAY set `tool_call_extra_content`, and the gateway SHALL add that object as `extra_content` to every assistant tool call it sends to that provider's endpoints. A request with no assistant tool call, and a provider without the setting, SHALL be unaffected.

#### Scenario: A Gemini-style provider gets the placeholder signature
- **WHEN** a Responses request replaying a function call and its output is routed to a provider whose `tool_call_extra_content` is `{ google = { thought_signature = "skip_thought_signature_validator" } }`
- **THEN** the upstream assistant message's tool call carries `extra_content.google.thought_signature` with that value

#### Scenario: Other providers are untouched
- **WHEN** the same request is routed to a provider without the setting
- **THEN** the tool call has no `extra_content`

#### Scenario: No replayed tool call
- **WHEN** the request has no assistant tool call
- **THEN** the body sent to a provider with the setting is the same as without it
