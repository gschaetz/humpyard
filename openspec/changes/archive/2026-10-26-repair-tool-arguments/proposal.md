# Proposal

## Why

On 2026-10-26 a Codex session on `deep-thinker-public` was rejected twice in a row, and the provider messages recorded since 0.18.0 say why: NVIDIA answered `messages[15].tool_calls[0].function.arguments must be a valid JSON object string: EOF while parsing an object`. An earlier assistant turn, served by a weak free model, ended with a tool call whose JSON arguments were cut off. The client stored it and replays it every turn, so every provider that checks arguments (NVIDIA, Google, probably OpenRouter) refuses every later turn, while lenient providers accept it. The 2026-10-25 stuck turn (three providers, "invalid argument") fits the same cause. The session cannot recover on its own and only a lenient pool, or a fresh session, gets past it.

The gateway sees the bad value on the way in and the repair is mechanical, so one cut-off call should not poison a whole conversation.

## What Changes

- Before a request is routed, every tool call in its history whose `arguments` is not a JSON object (a truncated object, text that is not JSON, an empty string, or JSON that is not an object) is replaced by one that is: a truncated object is completed (open string and brackets closed, a dangling key or comma dropped) when that yields an object, otherwise `{}`.
- A tool call whose arguments are already a valid JSON object is never touched, byte for byte. Only assistant tool calls are looked at; nothing else in the request changes.
- The repair applies to the parsed request and to a preserved same-format body (chat or Responses), because a preserved body is what would otherwise be sent as the client wrote it.
- A decision records how many tool calls were repaired (`repaired_tool_arguments`), shown by `GET /v1/decisions`, `GET /admin/v1/decisions` and `humpyard ctl decisions`, and absent when none was. Only the count is kept, never the arguments.
- Ledger schema v12: one nullable column on `decisions`. It is additive and one-way, like v7 to v11.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `upstream-proxy`: the request adaptation for chat-completions providers also repairs tool-call arguments that are not a JSON object.
- `decision-log`: a decision carries the number of tool calls whose arguments were repaired, and keeps nothing of the arguments.

## Impact

- Code: a small pure module for the repair (registered in the architecture test), the call in `server.rs` after the request is decoded, `metering.rs`, `ledger/decisions.rs` and `ledger.rs` (v12), `server/decisions.rs` and `ctl.rs` (display).
- Storage: one nullable integer per decision; retention is unchanged.
- API: one new optional field on a decision; existing fields are unchanged.
- Behaviour: a request that carried an invalid tool call now reaches strict providers with a valid one. The repaired call is a best effort (the cut-off text was lost either way); it is the history, not the answer, so the model sees a call that ran with partial or empty arguments.
- Operations: schema v12 cannot be undone, so back up `ledger.db` (with its `-wal` and `-shm`) before upgrading, as for 0.18.0.
- Out of scope: stopping a model from emitting a cut-off call (the response side), and changing which models serve a pool (a cluster-config matter handled separately).
