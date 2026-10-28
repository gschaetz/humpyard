## ADDED Requirements

### Requirement: An answer is judged for four facts
The gateway SHALL judge each upstream call that returns an answer for four facts, `empty_output`, `truncated_output`, `bad_tool_call` and `cut_stream`, and keep each as a flag. A flag that does not apply SHALL be unset, and a call the gateway could not observe SHALL read as unknown, never as unset.

#### Scenario: A good answer
- **WHEN** a call finishes normally with text, a normal stop and valid tool calls
- **THEN** it is observed and carries no flag

#### Scenario: A call that could not be observed
- **WHEN** a call returns no response to look at
- **THEN** its flags read as unknown, not as unset

### Requirement: An empty answer is flagged
`empty_output` SHALL be set when a call finished normally and its answer held no text and no tool call. Reasoning text alone SHALL NOT count as an answer.

#### Scenario: An empty answer
- **WHEN** a call finishes normally with no text and no tool call
- **THEN** it carries `empty_output` and no other flag

#### Scenario: A reasoning-only answer is empty
- **WHEN** a call finishes normally having produced reasoning text but no answer text and no tool call
- **THEN** it carries `empty_output`

### Requirement: An answer stopped by the length limit is flagged
`truncated_output` SHALL be set when the provider ended the answer because it reached its output limit, whether the answer was buffered or streamed. A stop reason the gateway does not recognize SHALL NOT set it.

#### Scenario: A length stop
- **WHEN** a provider ends an answer because it reached the output limit
- **THEN** the call carries `truncated_output`, in a buffered answer and in a streamed one

#### Scenario: An unknown stop reason
- **WHEN** a provider ends an answer with a stop reason the gateway does not know
- **THEN** the call does not carry `truncated_output`

### Requirement: A malformed tool call is flagged
`bad_tool_call` SHALL be set when an answer held a tool call with no name, or with arguments that are not blank and are not a JSON object. Blank arguments SHALL NOT set it, because providers send them for a tool that takes none.

#### Scenario: A cut-off tool call
- **WHEN** a tool call's arguments end in the middle of a JSON object
- **THEN** the call carries `bad_tool_call`

#### Scenario: A tool call with no arguments
- **WHEN** a tool call has a name and blank arguments
- **THEN** it does not carry `bad_tool_call`

#### Scenario: A tool call with no name
- **WHEN** a tool call arrives without a name
- **THEN** the call carries `bad_tool_call`

### Requirement: A stream the provider ended early is flagged
`cut_stream` SHALL be set when a streamed answer's provider stream ended or failed before its final chunk. It SHALL NOT be set when the client closed the connection first.

#### Scenario: A stream the provider cut off
- **WHEN** a provider's stream ends, or fails, before its final chunk
- **THEN** the call carries `cut_stream`

#### Scenario: A client that disconnects
- **WHEN** the client closes the connection while the provider is still streaming
- **THEN** the call does not carry `cut_stream`, because the provider did not end it

### Requirement: Observation never changes, delays or fails a response
Observing a response SHALL NOT alter any byte the client receives, SHALL NOT wait for anything the response did not already wait for, and SHALL NOT turn a successful response into a failure. A flag SHALL never influence which target serves a request.

#### Scenario: The client's bytes are the provider's
- **WHEN** a streamed answer carrying a malformed tool call is returned
- **THEN** the client receives the same events it would have received without the observation, and the call is flagged afterwards

#### Scenario: A failure to record
- **WHEN** the ledger cannot be written
- **THEN** the response is returned as it would have been and the flags are lost with the entry

### Requirement: Observation is bounded
The memory and time used to judge one response SHALL be bounded regardless of its size. Tool-call arguments SHALL be judged from at most a fixed number of calls and a fixed number of bytes per call, and a call past either bound SHALL NOT be flagged `bad_tool_call` for lack of a complete look.

#### Scenario: A very large argument
- **WHEN** a tool call's arguments exceed the byte bound
- **THEN** the call is not flagged `bad_tool_call` on account of them, and the response is delivered unchanged

### Requirement: Only whether a flag is set is kept
The gateway SHALL keep for each flag only that it is set. It SHALL NOT keep, log or expose any answer text, reasoning text, tool name or tool-call argument, or any part of them, and judging a response SHALL NOT write them anywhere.

#### Scenario: A flagged answer leaves no text
- **WHEN** a call with a malformed tool call whose arguments contain a recognizable string is recorded
- **THEN** the ledger, the decisions, the statistics, the metrics and the logs hold the flag and none of that string
