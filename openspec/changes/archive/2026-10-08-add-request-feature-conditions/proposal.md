# Proposal

## Why

Selector rules can route on who asks and what they say, but not on what the request contains.
Common flows depend on content: long prompts need a long-context tier, requests with images need
a vision-capable model, tool-using agent turns may want a different tier than plain chat.

## What Changes

- New rule conditions: `prompt_tokens = { min, max }` (inclusive bounds on a conservative local
  estimate of the prompt size, the same estimate `count_tokens` returns), `tools` (the request
  defines tools) and `images` (the request carries images).
- The estimate is computed only when some rule uses `prompt_tokens`.
- `explain` accepts `prompt_tokens`, `tools` and `images` for the hypothetical request and says
  when a rule failed on them ("wanted at least 2000, got 10"); an unspecified size never matches
  a `prompt_tokens` rule.
- Config validation: a `prompt_tokens` range needs a bound and `min` must not exceed `max`.
- README and docs updated.

## Capabilities

### New Capabilities

### Modified Capabilities
- `route-selectors`: adds content conditions.

## Impact

`src/config/*`, `src/select.rs`, `src/server.rs`, `src/server/explain.rs`, tests, docs.
