## ADDED Requirements

### Requirement: Response signals are counted
The gateway SHALL expose a counter of response signals set, labelled by route, target and a signal name drawn from the fixed set `empty_output`, `truncated_output`, `bad_tool_call` and `cut_stream`, and a counter of observed answer calls with the same route and target labels. A call that sets no signal SHALL add only to the observed counter. The labels SHALL follow the same bounded-vocabulary rules as the other series.

#### Scenario: A signal is counted
- **WHEN** a call served by target `a` on route `r` carries `cut_stream`
- **THEN** the signal counter for route `r`, target `a` and `cut_stream` rises by one, and so does the observed counter

#### Scenario: A clean call
- **WHEN** a call carries no signal
- **THEN** only the observed counter rises

#### Scenario: Labels stay bounded
- **WHEN** calls are served by targets that are not in the configuration
- **THEN** their series use the same bounded fallback label as other counters and the series count stays bounded
