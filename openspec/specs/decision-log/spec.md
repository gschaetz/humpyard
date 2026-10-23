# decision-log Specification

## Purpose
Defines the durable, queryable record of routing decisions: one row per request, including refusals and cancellations, with attempts, joined to usage by request id, readable per key, metadata only and pruned by retention.

## Requirements

### Requirement: Every request has an id
The gateway SHALL give each inference request an id, return it in `x-humpyard-request-id` (on errors too), and store it on the usage rows the request produced.

#### Scenario: Join decision and usage
- **WHEN** a request succeeds
- **THEN** its decision row and its usage row(s) carry the same request id, equal to the response header

### Requirement: One decision row per request, including refusals
For every inference request the gateway SHALL record a decision with the requested model, selected route and rule, the budget state, the ordered upstream attempts with their outcomes, the final status, and the serving target; requests refused before routing (authentication, authorization, unknown model, budget, no eligible target) SHALL be recorded with their reason.

#### Scenario: Fallback is visible
- **WHEN** the first target fails and the second serves
- **THEN** the decision lists both attempts in order, with the first marked as failed, and the serving target is the second

#### Scenario: Refusal recorded
- **WHEN** a key is refused for a route it may not use
- **THEN** a decision exists with outcome `forbidden` and no attempts

### Requirement: Recording never delays or fails a request
Decisions SHALL be written by the asynchronous ledger writer; a full queue or failing database SHALL be counted and logged, never surfaced to clients, and queued decisions SHALL be flushed on graceful shutdown.

#### Scenario: Database failure
- **WHEN** the ledger database cannot be written
- **THEN** requests still succeed

### Requirement: Decisions are readable per key
`GET /v1/decisions` SHALL return the caller's own decisions, newest first, filtered by `route`, `rule`, `outcome`, `request_id` and `since_ms`, with a bounded `limit`; an open gateway SHALL return all decisions.

#### Scenario: Isolation between keys
- **WHEN** two keys have made requests
- **THEN** each sees only its own decisions

### Requirement: Metadata only, with retention
Decisions SHALL NOT contain prompts or responses, SHALL truncate error text, and SHALL be deleted after `decision_retention_days` (default 90; 0 keeps them).

#### Scenario: Old rows pruned
- **WHEN** a decision is older than the retention window
- **THEN** it is removed by the writer's periodic pruning

### Requirement: Decisions and explain show the overrides that applied
Each decision SHALL list the overrides that applied to it (id, kind and subject), and `explain` SHALL show a target removed by an override with the override's id as the reason. The effective configuration view SHALL label a target disabled by an active override with the source `override`.

#### Scenario: An override is visible
- **WHEN** a request is planned while a target is disabled
- **THEN** its decision lists the override and `explain` names it as the reason the target was removed
