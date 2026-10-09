# Proposal

## Why

A real Codex session against a hosted chat-completions provider (first daily-use trial, 2026-10-08)
failed twice mid-task with upstream 400s that only appear once a conversation has some history:
1. `reasoning_details.summary ... array is not acceptable`: Codex replays prior reasoning items,
   and the translation forwards the whole Responses item as a chat "reasoning detail".
2. `role 'developer' is not allowed`: Codex injects developer-role messages (the decoder hoists
   them into the instructions, and the chat encoder writes them back as `developer`).

Earlier short Codex runs passed because they never replayed such items. Both are protocol
mismatches at the Responses-to-chat boundary, not provider quirks.

## What Changes

Before a request is sent to any endpoint (all endpoints speak chat completions) the pool:
- drops reasoning details shaped like Responses reasoning items (`type: "reasoning"`), keeping the
  reasoning text and any genuine chat-style details (for example `reasoning.encrypted`);
- sends the `developer` role as `system`, in both the instructions and the messages.

## Capabilities

### New Capabilities

### Modified Capabilities
- `upstream-proxy`: requests are adapted for chat-completions providers.

## Impact

`src/pool.rs`, new `tests/reasoning_replay.rs`, docs/clients.md.
