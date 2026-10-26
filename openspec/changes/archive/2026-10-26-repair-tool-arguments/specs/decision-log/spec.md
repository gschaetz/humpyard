## ADDED Requirements

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
