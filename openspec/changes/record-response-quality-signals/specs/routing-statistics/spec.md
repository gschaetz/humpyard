## ADDED Requirements

### Requirement: Statistics report the response signals per target
`GET /v1/stats` and its admin counterpart SHALL report, for each group over the requested window, how many answer calls were observed and how many carried each response signal, so a rate is the count of a signal over the observed calls. A call whose signals are unknown SHALL count in neither figure. The figures SHALL follow the same isolation and window rules as the other statistics.

#### Scenario: A rate can be computed
- **WHEN** a target served ten observed calls and two of them carried `truncated_output`
- **THEN** its group reports ten observed calls and two truncated

#### Scenario: Unknown calls are left out
- **WHEN** a window holds calls written before the signals existed
- **THEN** they do not count as observed and do not lower any rate

#### Scenario: Isolation
- **WHEN** two keys have made requests
- **THEN** each key's response-signal figures count only its own calls
