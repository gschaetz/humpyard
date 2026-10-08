# Spec Delta

## MODIFIED Requirements

### Requirement: Streaming usage recorded at stream end
For streamed responses the gateway SHALL record usage when the stream ends. If the stream ends without usage, the gateway SHALL record the entry with the missing-usage marker and, when text was generated and delivered, an output token estimate (and its cost) derived from the delivered bytes; input tokens SHALL stay zero.

#### Scenario: Normal stream
- **WHEN** a streamed answer completes with a usage chunk
- **THEN** its entry carries those token counts

#### Scenario: Stream without usage
- **WHEN** the provider reports no usage for a stream that delivered text
- **THEN** the entry is recorded with the missing-usage marker, an estimated positive output count and its cost, and a warning is logged

#### Scenario: Cut stream
- **WHEN** a stream is cut by the grace period or the client disconnects after text was delivered
- **THEN** the entry is `cancelled`, marked as an estimate, with the estimated output tokens

#### Scenario: Nothing generated
- **WHEN** a stream ends without usage and without any generated text
- **THEN** the entry has zero tokens and zero cost
