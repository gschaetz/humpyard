# Spec Delta

## ADDED Requirements

### Requirement: Provider headers
The configuration SHALL allow each provider an optional `headers` table of additional HTTP headers, and SHALL reject header names or values that are not valid HTTP and names that would override authentication.

#### Scenario: Valid headers
- **WHEN** a provider declares `headers = { "x-app" = "conductor" }`
- **THEN** the configuration loads

#### Scenario: Authentication header rejected
- **WHEN** a provider declares a header named `authorization` or `x-api-key`
- **THEN** the process exits non-zero naming the provider and the header

#### Scenario: Invalid header name
- **WHEN** a provider declares a header whose name contains whitespace
- **THEN** the process exits non-zero naming the provider and the header
