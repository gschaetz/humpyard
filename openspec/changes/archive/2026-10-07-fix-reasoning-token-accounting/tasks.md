# Tasks

## 1. Fix and pin

- [x] 1.1 Remove reasoning tokens from `cost_micro_usd`, `total_tokens` and `Entry::total_tokens`, and from the ledger's spend sums; update the pricing and ledger unit tests and the property tests, and add a property that reasoning tokens never change cost or totals; verify `cargo test` passes
- [x] 1.2 Add an end-to-end usage test with a mock provider reporting `completion_tokens_details.reasoning_tokens` inside `completion_tokens`, asserting the ledger entry, cost and key-info token total use the output count once; and a codec test pinning that Switchyard's OpenAI Chat decoder reports reasoning as a subset (so an upgrade that changes it fails)
- [x] 1.3 Correct `docs/budgets.md` and the pricing doc comment; verify no remaining text says reasoning bills in addition to output
