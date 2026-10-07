# Spec Delta

## ADDED Requirements

### Requirement: Configured provider headers are sent
The gateway SHALL send each provider's configured headers on every upstream call to that provider, and only to that provider.

#### Scenario: Header on every call
- **WHEN** a provider configures `x-app = "conductor"` and a request is routed to it
- **THEN** the upstream request carries `x-app: conductor`

#### Scenario: Not sent to other providers
- **WHEN** a request fails over from provider A (with headers) to provider B (without)
- **THEN** provider B's request does not carry A's headers
