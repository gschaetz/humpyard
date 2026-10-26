# Proposal

## Why

On 2026-10-25 a Codex session received three HTTP 400s that every provider had rejected (openrouter, nvidia and googleai, each within a second), and the decision log recorded only `rejected` for each attempt. The upstream message is the one fact that says why, and it is thrown away: only the final error's text is kept, so with failover the reason for every earlier attempt is lost. The cause could not be found without guessing, and a stuck session keeps failing until someone knows what to change.

## What Changes

- Each upstream attempt on a decision gains an optional `message`: the provider's own error text for a failed attempt, scrubbed and cut to a fixed length. A successful attempt has none.
- Scrubbing removes anything key-shaped (bearer tokens, `sk-`-style and long random strings), control characters and line breaks before the text is stored, and the stored text never exceeds the limit decisions already use for error text.
- `GET /v1/decisions`, `GET /admin/v1/decisions` and `humpyard ctl decisions` show the message with each attempt; nothing else about a decision changes.
- No new column and no schema migration: attempts are already stored as a JSON list on the decision, so an old row simply has attempts with no message and a new binary reads both.
- No prompt or response text is stored. Only the provider's error message is kept, which is the same kind of text a decision's `message` field already holds.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `decision-log`: an attempt can carry the provider's scrubbed, bounded error message, and the "metadata only" requirement states what may and may not be kept.

## Impact

- Code: `pool.rs` (the attempt report), `metering.rs` and `ledger/decisions.rs` (the `Attempt` record), a small scrubber, `server/decisions.rs` and `ctl.rs` (display). No new dependency.
- Storage: decisions grow by at most the message limit per failed attempt. Retention is unchanged, so rows are pruned with their decision.
- API: one new optional field on each attempt in the decision list; existing fields are unchanged.
- Risk: a provider's error text can echo a fragment of the request. The limit and the scrubbing bound this, and the design records what remains.
