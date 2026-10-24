# Design

## Context

The OpenAI-compatible endpoint of Gemini 3.x needs `extra_content.google.thought_signature` on each replayed
function call. Switchyard's `ToolCall` has no slot for provider data, so the real signature is lost on decode and
cannot round-trip through a Responses client either. A stateful side channel (a cache of signatures by call id) would
need the response decoder to surface the value, which it does not.

## Decisions

- **A configured static value, per provider.** `tool_call_extra_content` is a table added as `extra_content` to
  each replayed assistant tool call lacking one. For Google AI the table is the documented
  `skip_thought_signature_validator` placeholder. It is generic (no Google-specific code) and explicit, like
  `drop_fields`; nothing is keyed on a provider's name.
- **How it is applied.** The translation client builds the body with the public `encode_request` and replays a
  preserved same-format body verbatim. For an endpoint with the setting, and only when the request has an assistant
  tool call, the pool encodes the chat body the same way, adds the content to each tool call, and stores the result as
  the preserved chat body, so the client sends exactly that (it still sets the model and the stream-usage option
  afterwards). The preserved body is cleared first, so a chat client's request is rebuilt from the normalized
  request for such an endpoint.
- **Per attempt, after `drop_fields`.** A failover to a provider without the setting is unaffected.

## Risks

The placeholder skips Google's validation and may cost a little quality on later turns; the real signature would be
better. If Switchyard later carries provider data on tool calls, a real round trip should replace this for Gemini.
The pool re-encodes with the same helper the client uses; a Switchyard upgrade that changes that is caught by the
test, which compares against the exact upstream body.

## Out of scope

Carrying real signatures (needs Switchyard support); failover on a provider-specific 400.
