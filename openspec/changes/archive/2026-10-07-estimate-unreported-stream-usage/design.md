# Design

## Decisions

- **Estimate from delivered bytes** of text, reasoning and tool-call arguments, using the
  `estimate` ratio (ceil bytes/3), which errs high: for a spend limit, over-counting a cut stream
  is the safer error than counting it as free.
- **Output only.** The prompt is not available at the tap, and guessing it would double-handle
  data the gateway otherwise never inspects. Input cost of a cut stream stays uncounted; recorded
  as a limitation.
- **Reuse `usage_missing`** as the estimate marker rather than a schema change; its meaning
  ("the provider did not report usage") is unchanged.

## Risks / Trade-offs

- The estimate can exceed the provider's bill by up to ~2x for English prose.
- Bytes delivered to the gateway are counted, not bytes the client received.
