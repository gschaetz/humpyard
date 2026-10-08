# Design

## Context

`POST /v1/messages/count_tokens` is answered locally by `server::count_tokens` using
`estimate_tokens` over the chat-completions form of the request (change add-count-tokens). The
Responses twin differs only in the request protocol (decode as Responses), the success body, and
the error shape.

## Decisions

- **One handler, two protocols.** The counting handler takes the wire format as a parameter and the
  two routes bind it. Decode, authorize and the estimate are unchanged; only the success body
  differs.
- **Do not build `/v1/responses/compact`.** Evidence (Codex source, 2026-10-08): a provider's
  `remote_compaction` capability defaults to `Unsupported` unless it is OpenAI or Azure Responses
  (`ProviderCapabilities::from_config`); other providers compact locally by summarizing through
  ordinary Responses calls. The endpoint also returns OpenAI-specific encrypted compaction items
  that must round-trip into later `input` arrays, which our Chat-based upstreams cannot honor.
  There is no consumer and a large translation cost. To revisit: a user can force it with Codex's
  `capabilities.remote_compaction` override, at which point a real need exists; the likely design is
  to summarize with a model and return the summary as an opaque item, converting it back to text
  on the way upstream.

## Risks / Trade-offs

- Same estimate caveats as `count_tokens` (conservative, not exact).
