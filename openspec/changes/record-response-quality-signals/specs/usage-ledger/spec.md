## ADDED Requirements

### Requirement: A call's entry carries the response signals
Each ledger entry for an answer call SHALL carry the four response signals (`empty_output`, `truncated_output`, `bad_tool_call`, `cut_stream`) for the call it describes, attributed to that entry's target, provider and model. A signal SHALL read as unknown, not as unset, on an entry written before the signals existed, on a judge call, and on a call whose response was not observed. Entries SHALL keep the signals for as long as they keep the entry.

#### Scenario: A flagged call
- **WHEN** a request is served by target `a` and its answer ended on the length limit
- **THEN** the entry for that call names target `a` and carries `truncated_output`

#### Scenario: A clean call
- **WHEN** a request is served and its answer is whole
- **THEN** the entry carries all four signals as unset, not as unknown

#### Scenario: Older rows
- **WHEN** an entry written before this requirement is read
- **THEN** its signals read as unknown and nothing fails

#### Scenario: A streamed answer is flagged when it ends
- **WHEN** a streamed answer ends after its decision was recorded
- **THEN** the entry written at the end of the stream carries its signals

#### Scenario: A judge call
- **WHEN** a routing-time judge call is recorded
- **THEN** its signals read as unknown
