# Spec Delta

## ADDED Requirements

### Requirement: Upstream error messages are readable
When an upstream error is passed to the client or logged, the gateway SHALL use the provider's JSON error message when one is present, and otherwise a short status-based message that never includes HTML or unrecognized JSON bodies.

#### Scenario: HTML error page
- **WHEN** an upstream answers 404 with an HTML page
- **THEN** the client's message is `upstream returned 404 Not Found`

#### Scenario: Provider JSON message
- **WHEN** the upstream body has `error.message`
- **THEN** that message is passed through
