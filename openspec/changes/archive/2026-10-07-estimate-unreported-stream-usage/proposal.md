# Proposal

## Why

A real run (2026-10-08) cut a streamed OpenCode Go answer at the grace period: about 36 KB had been
delivered, yet the ledger row had zero tokens and zero cost, because a cut stream never reaches the
provider's final usage chunk. Budgets under-count exactly the spend they exist to bound.

## What Changes

- When a stream ends (cut, dropped client, or provider that never reports usage) with no usage but
  with generated text, the gateway records an **output-only estimate** from the bytes delivered
  (same conservative bytes-per-token ratio as token counting) and prices it. Input is not guessed.
  The row keeps `usage_missing = 1` to mark it an estimate.
- Streams that produced nothing still record zero.

## Capabilities

### New Capabilities

### Modified Capabilities
- `usage-ledger`: streams without reported usage record an output estimate instead of zero.

## Impact

`src/metering.rs`, `src/estimate.rs` (one function made public), architecture rules (metering may
use estimate), tests, docs.
