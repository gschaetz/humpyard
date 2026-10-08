# Design

## Context

`pricing::cost_micro_usd` billed `output + reasoning` at the output price, `pricing::total_tokens`
and `Entry::total_tokens` added reasoning to the total, and the ledger's spend query summed
`output_tokens + reasoning_tokens`. Evidence: Switchyard 0.3.0's OpenAI Chat decoder
(`decode_openai_usage`, and the stream equivalent) sets `output_tokens = completion_tokens` and
`reasoning_tokens = completion_tokens_details.reasoning_tokens` without subtracting; the Anthropic
and Responses decoders do the same with `thinking_tokens` / `reasoning_tokens`. All three APIs define
reasoning as a subset of output.

## Decisions

- **Subset semantics, no heuristics.** Output tokens already include reasoning; the reasoning column
  is detail only. A heuristic such as "add reasoning if it exceeds output" would hide provider
  quirks rather than surface them. A provider that reports reasoning *separately from* completion
  tokens would be under-counted; that is a documented limitation to handle with a per-provider
  option if one appears.
- **Pin the assumption.** A test decodes a real-shaped usage payload through Switchyard's codec and
  asserts the subset relationship, so an upgrade that changes it fails loudly.
- **Keep the reasoning column.** It is useful detail (cost of thinking) and removing a column would
  need a migration for no gain.

## Risks / Trade-offs

- Previously recorded ledger rows may overstate cost for reasoning calls; not corrected (pre-alpha).
