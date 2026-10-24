# Spec Delta

## ADDED Requirements

### Requirement: A provider may name fields to drop
A provider MAY set `drop_fields` to a list of non-empty strings. An empty name SHALL be refused when the configuration loads, and the absence of the setting SHALL mean no fields are removed.

#### Scenario: A valid list
- **WHEN** a provider sets `drop_fields = ["store", "prompt_cache_key"]`
- **THEN** the configuration loads

#### Scenario: An empty name
- **WHEN** a provider's `drop_fields` contains an empty string
- **THEN** the configuration is refused naming the provider
