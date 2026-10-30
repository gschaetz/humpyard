## ADDED Requirements

### Requirement: A provider's timeout bounds the wait for a response to start
A provider's `timeout_secs` SHALL bound the time the gateway waits for that provider to begin a response, counted from the start of the call and including the provider's retries. When it passes before the response has begun, the call SHALL fail with a timeout, which fails over to the next endpoint or target exactly as any other timeout does.

#### Scenario: A provider that never starts
- **WHEN** a provider sends nothing for longer than its `timeout_secs`
- **THEN** the call fails with a timeout and the request moves to the next endpoint or target

#### Scenario: A provider that answers in time
- **WHEN** a provider begins its response within `timeout_secs`
- **THEN** the wait ends and no timeout is raised for the time that follows

### Requirement: A started stream is limited by idle time, not by its length
Once a streamed answer has begun, `timeout_secs` SHALL limit how long the stream may send nothing, counted from the last chunk received. A stream that sends nothing for `timeout_secs` SHALL end with the timeout error. A stream that keeps sending chunks SHALL NOT be ended because of its total length, however long it runs.

#### Scenario: A long stream that keeps producing
- **WHEN** a streamed answer runs for longer than `timeout_secs` in total and a chunk arrives within every `timeout_secs`
- **THEN** it completes and the client receives the whole answer

#### Scenario: A stream that stalls
- **WHEN** a started stream sends nothing for `timeout_secs`
- **THEN** it ends with the timeout error, and the failure is recorded as a timeout

#### Scenario: A slow start followed by a healthy stream
- **WHEN** a provider takes most of `timeout_secs` to begin a stream and then sends chunks steadily
- **THEN** the stream is not ended when the original wait would have run out

### Requirement: A buffered answer is bounded as a whole
A response that is not streamed has no earlier point at which it starts, so `timeout_secs` SHALL bound the whole call, retries included, for it.

#### Scenario: A buffered answer that takes too long
- **WHEN** a non-streamed call does not finish within `timeout_secs`
- **THEN** it fails with a timeout and fails over as before

### Requirement: A timeout says which limit was reached
The timeout error SHALL say whether the wait for the response to start, the idle time of a stream, or the whole of a buffered call ran out, with the configured limit, so a log line or a recorded attempt can be read without the configuration at hand. The failure class SHALL stay `timeout` in every case.

#### Scenario: A stalled stream is logged with its reason
- **WHEN** a started stream is ended for being idle
- **THEN** the failure message names the idle limit and the failure class is `timeout`
