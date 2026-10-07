# Spec Delta

## MODIFIED Requirements

### Requirement: Serving attribution
The gateway SHALL report the target and provider that served each non-streaming and streaming response in `x-humpyard-target` and `x-humpyard-provider` response headers, and in the request log.

#### Scenario: Header present
- **WHEN** a request is served
- **THEN** both headers name the serving target and provider
