# Spec Delta

## MODIFIED Requirements

### Requirement: Model listing
The gateway SHALL answer `GET /v1/models` with an OpenAI-format list containing every configured route and target name.

#### Scenario: List models
- **WHEN** a client requests `/v1/models`
- **THEN** the response lists each route and target name from the configuration
