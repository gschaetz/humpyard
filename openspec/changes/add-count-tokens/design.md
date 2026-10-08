# Design

## Context

Providers are OpenAI-compatible chat endpoints with no token-count call. The only exact count is
the `prompt_tokens` a provider reports *after* a completion. Switchyard's server answers
`count_tokens` only for routes with an Anthropic backend (`AuxiliaryUnsupported` otherwise).

## Decisions

- **Local estimate, not a probe.** A probe (sending the prompt with `max_tokens: 1` to read
  `prompt_tokens`) is exact but bills the whole prompt on every call, and clients may call this
  often. An estimate is free and instant. A probe mode can be added later as an option.
- **Estimate the wire form the provider sees.** Decode the request, encode it as an OpenAI Chat
  body, and measure the `messages` and `tools` it contains: this includes system prompts, tool
  schemas and tool-call arguments, which dominate agent prompts. The estimator is a pure function
  over JSON (`estimate_tokens(&Value)`), kept free of Switchyard types so it lives in a core module.
- **Conservative ratio.** `ceil(utf8 bytes / BYTES_PER_TOKEN)` plus a small per-message overhead.
  The constant is chosen from a calibration against real providers (see Findings), erring high.
- **Never blocked by budget.** No provider call and no ledger entry, so no spend to protect.
- **Access rules shared with `/v1/messages`**: authentication, allowlist, unknown model, and
  Anthropic-shaped errors reuse the existing stages; the policy and budget stages are skipped.
- **Marked as an estimate** by a response header, not a body field, so strict clients still see
  exactly Anthropic's shape.

## Risks / Trade-offs

- Different models tokenize differently; an estimate can be off by tens of percent either way.
  Erring high makes clients compact slightly early, never overflow silently.
- Clients that use the count to bill or limit precisely get an approximation; documented.

## Findings

(filled in during implementation: calibration against real providers)
