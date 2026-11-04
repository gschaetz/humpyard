## MODIFIED Requirements

### Requirement: Requests are adapted for chat-completions providers
Before sending a request upstream the gateway SHALL remove Responses-shaped reasoning details (entries of type `reasoning`) while keeping the reasoning text and chat-style details, SHALL send the `developer` role as `system`, and SHALL make every assistant tool call's arguments a JSON object. Arguments that already are a JSON object SHALL be sent exactly as the client wrote them. Arguments that are not (a truncated object, text that is not JSON, an empty string, or JSON of another kind) SHALL be replaced: a truncated object is completed when that yields a JSON object, and anything else becomes `{}`. The repair SHALL apply to a preserved same-format request body as well as to the decoded request, SHALL change nothing else in the request, and SHALL NOT depend on the provider or the route.

#### Scenario: Replayed reasoning item
- **WHEN** a Responses request replays a reasoning item that carries `content` or `encrypted_content`
- **THEN** the upstream request has no Responses-shaped `reasoning_details` and still carries the reasoning text

#### Scenario: Chat-style details are untouched
- **WHEN** a chat request carries `reasoning_details` of type `reasoning.encrypted`
- **THEN** they reach the provider unchanged

#### Scenario: Developer messages
- **WHEN** a request contains developer-role messages, at the start or mid-conversation
- **THEN** the upstream request has no `developer` role and the same number of system messages

#### Scenario: A cut-off tool call in the history is completed
- **WHEN** a Responses or chat request replays an assistant tool call whose arguments are `{"cmd":["ls","-la"],"workdir":"/tm`
- **THEN** the upstream request carries that call with arguments that are a valid JSON object holding `cmd` and `workdir`, and a provider that rejects invalid arguments accepts the request

#### Scenario: Arguments that cannot be completed become an empty object
- **WHEN** an assistant tool call's arguments are not JSON at all, are empty, or are JSON that is not an object
- **THEN** the upstream request carries `{}` for that call

#### Scenario: Valid arguments are untouched
- **WHEN** an assistant tool call's arguments are already a JSON object, including one with unusual spacing or key order
- **THEN** the upstream request carries them unchanged and nothing is counted as repaired

#### Scenario: A preserved same-format body is repaired too
- **WHEN** a chat request is forwarded as the body the client sent, and one of its tool calls holds cut-off arguments
- **THEN** the body sent upstream holds the repaired arguments and every other part of it is as the client sent it

#### Scenario: Only tool-call arguments are touched
- **WHEN** a request holds text, tool results or tool definitions that look like broken JSON
- **THEN** they reach the provider unchanged
