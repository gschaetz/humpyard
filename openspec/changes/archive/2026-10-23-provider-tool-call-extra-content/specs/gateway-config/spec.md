# Spec Delta

## ADDED Requirements

### Requirement: A provider may set extra content for tool calls
A provider MAY set `tool_call_extra_content` to a table. A value that is not a table SHALL be refused when the configuration loads, and its absence SHALL mean tool calls are sent unchanged.

#### Scenario: A valid table
- **WHEN** a provider sets `tool_call_extra_content = { google = { thought_signature = "skip_thought_signature_validator" } }`
- **THEN** the configuration loads

#### Scenario: Not a table
- **WHEN** a provider sets `tool_call_extra_content = "x"`
- **THEN** the configuration is refused naming the provider
