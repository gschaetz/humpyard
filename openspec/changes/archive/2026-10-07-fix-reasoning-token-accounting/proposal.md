# Proposal

## Why

The first run against a real provider (OpenCode Go, `glm-5.3-flash`) showed our accounting
disagreeing with the provider's own numbers: the provider reported `completion_tokens: 150` and
`total_tokens: 167`, with 150 of the completion tokens being reasoning tokens. The ledger recorded
317 tokens and a cost computed on 300 output tokens, because we added reasoning tokens to output
tokens. In the OpenAI Chat, OpenAI Responses and Anthropic Messages APIs, reasoning (thinking)
tokens are a *subset* of the completion/output tokens, so adding them double counts. Reasoning-heavy
responses were over-billed and over-counted against budgets by up to 2x.

## What Changes

- Cost and token totals use output tokens only; reasoning tokens stay recorded for information
  and are never added to cost or to token budgets.
- Ledger totals (the SQL sums that rebuild budgets after a restart) follow the same rule.
- Docs corrected (they said reasoning tokens "bill as output" in addition).
- Switchyard's own doc comment ("output tokens excluding reasoning") is wrong for these codecs; we
  rely on observed behavior, pinned by a test against its decoder.

Out of scope: correcting already-recorded ledger rows (pre-alpha, no production data).

## Capabilities

### New Capabilities

### Modified Capabilities
- `usage-ledger`: reasoning tokens are part of output and are never counted twice.

## Impact

`src/pricing.rs`, `src/ledger.rs`, tests, `docs/budgets.md`. Costs and token budgets for reasoning
models become correct (lower than before for reasoning-heavy calls).
