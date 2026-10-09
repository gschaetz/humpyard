# Spec Delta

## ADDED Requirements

### Requirement: Requests are adapted for chat-completions providers
Before sending a request upstream the gateway SHALL remove Responses-shaped reasoning details (entries of type `reasoning`) while keeping the reasoning text and chat-style details, and SHALL send the `developer` role as `system`.

#### Scenario: Replayed reasoning item
- **WHEN** a Responses request replays a reasoning item that carries `content` or `encrypted_content`
- **THEN** the upstream request has no Responses-shaped `reasoning_details` and still carries the reasoning text

#### Scenario: Chat-style details are untouched
- **WHEN** a chat request carries `reasoning_details` of type `reasoning.encrypted`
- **THEN** they reach the provider unchanged

#### Scenario: Developer messages
- **WHEN** a request contains developer-role messages, at the start or mid-conversation
- **THEN** the upstream request has no `developer` role and the same number of system messages
