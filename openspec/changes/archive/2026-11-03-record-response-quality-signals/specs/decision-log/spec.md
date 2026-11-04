## ADDED Requirements

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
