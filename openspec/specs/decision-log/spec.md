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
Decisions SHALL NOT contain prompts or responses, SHALL keep error text only as scrubbed, truncated messages (a decision's own message and each failed attempt's provider message), and SHALL be deleted after `decision_retention_days` (default 90; 0 keeps them). The messages SHALL be removed with their decision.

#### Scenario: Old rows pruned
- **WHEN** a decision is older than the retention window
- **THEN** it is removed by the writer's periodic pruning, with its attempts and their messages

#### Scenario: No request content is kept
- **WHEN** a request fails at a provider
- **THEN** no part of the request body is stored, only the provider's scrubbed, truncated error text

### Requirement: Decisions and explain show the overrides that applied
Each decision SHALL list the overrides that applied to it (id, kind and subject), and `explain` SHALL show a target removed by an override with the override's id as the reason. The effective configuration view SHALL label a target disabled by an active override with the source `override`.

#### Scenario: An override is visible
- **WHEN** a request is planned while a target is disabled
- **THEN** its decision lists the override and `explain` names it as the reason the target was removed

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

### Requirement: A decision counts the tool calls whose arguments were repaired
A decision SHALL record how many assistant tool calls in the request had their arguments repaired before it was routed, and `GET /v1/decisions`, `GET /admin/v1/decisions` and `humpyard ctl decisions` SHALL show the count when it is more than zero. A request that needed no repair SHALL show nothing. The decision SHALL keep only the count: no part of the arguments, original or repaired, SHALL be stored. A decision written before this requirement existed SHALL read as before, with no count.

#### Scenario: A repair is counted
- **WHEN** a request with two cut-off tool calls in its history is routed
- **THEN** its decision shows a count of 2 and holds no text from either call's arguments

#### Scenario: No repair, no field
- **WHEN** every tool call in a request has valid arguments, or the request has none
- **THEN** its decision shows no count

#### Scenario: A refused request still records it
- **WHEN** a request that needed repair is refused (for example by a key's budget) before any provider is tried
- **THEN** its decision still shows the count

#### Scenario: Older rows still read
- **WHEN** a decision written before this change is read
- **THEN** it is returned as before, with no count, and nothing fails

### Requirement: A decision shows the response signals of the attempt that served it
`GET /v1/decisions`, `GET /admin/v1/decisions` and `humpyard ctl decisions` SHALL show, for a request that was served, the names of the response signals set on the call that served it, and SHALL show nothing when none was set or when they are unknown. The signals SHALL be read from the usage ledger, so a streamed answer's signals appear once the stream has ended. A decision SHALL still be returned, without them, while the stream is open.

#### Scenario: A flagged answer is visible
- **WHEN** the call that served a request carries `bad_tool_call`
- **THEN** its decision lists `bad_tool_call` among the response signals, on all three surfaces

#### Scenario: A clean answer shows nothing
- **WHEN** the call that served a request carries no flag
- **THEN** its decision shows no response signals

#### Scenario: A stream still open
- **WHEN** a decision is read while its answer is still streaming
- **THEN** it is returned without response signals and nothing fails

#### Scenario: Older decisions
- **WHEN** a decision written before this change is read
- **THEN** it is returned as before, with no response signals

### Requirement: A decision records its actor and owner
A decision SHALL record the id of the actor its key names and the id of that actor's owner, as they were when the request was made. The owner of a human actor SHALL be recorded as that human. A decision whose key names no actor, or an agent with no owner, SHALL record none for the missing id.

#### Scenario: An agent with an owner
- **WHEN** a request is made with a key whose actor is agent `openclaw` owned by `gary`
- **THEN** its decision records actor `openclaw` and owner `gary`

#### Scenario: A key of a human
- **WHEN** a request is made with a key whose actor is human `gary`
- **THEN** its decision records actor `gary` and owner `gary`

#### Scenario: No actor
- **WHEN** a request is made with a key that names no actor, or without a key in open mode
- **THEN** its decision records no actor and no owner

#### Scenario: History does not change
- **WHEN** an actor's owner is changed in the file and the gateway reloads
- **THEN** decisions made before the reload keep the owner they recorded

### Requirement: A decision's actor is not an authority
The actor and owner on a decision SHALL be recorded for attribution only and SHALL NOT be read to authorize, limit or route any request.

#### Scenario: Attribution only
- **WHEN** two requests differ only in the actor their keys name
- **THEN** routing, budgets and limits treat them identically
