## ADDED Requirements

### Requirement: A failed attempt records why the provider refused it
Each failed upstream attempt on a decision SHALL carry the provider's own error message, scrubbed and bounded, so the reason for every attempt in a failover chain is visible and not only the last one's. `GET /v1/decisions`, `GET /admin/v1/decisions` and `humpyard ctl decisions` SHALL show it with the attempt. A successful attempt, or one whose provider gave no usable text, SHALL carry none.

#### Scenario: Every rejection in a chain is explained
- **WHEN** three providers reject a request in turn
- **THEN** the decision lists the three attempts, each with its own provider's message

#### Scenario: A secret in an error is not stored
- **WHEN** a provider's error text contains a bearer token or a key-shaped string
- **THEN** the stored message has it removed and the rest of the text is kept

#### Scenario: Control characters and line breaks are removed
- **WHEN** a provider's error text spans several lines or holds control characters
- **THEN** the stored message is one line without them

#### Scenario: The message is bounded
- **WHEN** a provider returns a very long error body
- **THEN** the stored message is cut to the same limit decisions apply to error text

#### Scenario: A success has no message
- **WHEN** an attempt succeeds
- **THEN** it carries no message

#### Scenario: Older rows still read
- **WHEN** a decision written before this change is read
- **THEN** its attempts are returned as before, with no message, and nothing fails

## MODIFIED Requirements

### Requirement: Metadata only, with retention
Decisions SHALL NOT contain prompts or responses, SHALL keep error text only as scrubbed, truncated messages (a decision's own message and each failed attempt's provider message), and SHALL be deleted after `decision_retention_days` (default 90; 0 keeps them). The messages SHALL be removed with their decision.

#### Scenario: Old rows pruned
- **WHEN** a decision is older than the retention window
- **THEN** it is removed by the writer's periodic pruning, with its attempts and their messages

#### Scenario: No request content is kept
- **WHEN** a request fails at a provider
- **THEN** no part of the request body is stored, only the provider's scrubbed, truncated error text
