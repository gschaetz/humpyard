# Spec Delta

## MODIFIED Requirements

### Requirement: Policy context
The policy SHALL receive the requested route, session id and request metadata, and, when the caller is authenticated, the key id and its budget state, so it can decide per caller.

#### Scenario: Session-aware policy
- **WHEN** a policy decides by session
- **THEN** it can read the session id of the current request

#### Scenario: Budget-aware policy
- **WHEN** an authenticated key is in the restricted state
- **THEN** the policy can read the key id and that state
