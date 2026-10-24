# Design

## Context

`adapt_for_chat_providers` (src/pool.rs) runs once per target, before the endpoint walk, and cannot know
which provider will serve. The translation puts the Responses request's `prompt_cache_key` and `store` into
`LlmRequest.extensions.fields`, and the chat encoder copies them into the body. Providers differ: Google AI
rejects both with a 400, while others accept `prompt_cache_key` and use it for cache routing.

## Decisions

- **Per provider, in config, not by name.** `drop_fields` is an explicit list on the provider. No built-in
  default keyed on a provider's name: that would be hidden behaviour, and a provider's strictness can change.
  Operators who use Google AI add the two fields (documented in docs/providers.md).
- **Applied per attempt.** The endpoint loop already clones the request for each attempt to set the model, so
  the removal happens on that clone, and a failover to a provider without the setting keeps the field.
- **Extensions and the preserved body.** The removal edits `extensions.fields`. For a same-format request the
  codec prefers the preserved original body (`PreservationMetadata`), so the same names are removed from the top
  level of each preserved request body. Other preserved content is left as it was.
- **Scope.** Only pass-through fields can be dropped. Fields the gateway models (`messages`, `tools`, `stream`,
  ...) are not touched; naming one is a no-op, not an error, because the list is about what a provider rejects,
  not a schema.
- **Error arrays.** `upstream_message` also accepts a JSON array and reads the first element's
  `error.message`. No other shape changes.

## Risks

A name in `drop_fields` that a provider needs (for example `stream_options`, which carries usage) would break
metering for that provider. The documentation names the two fields Google AI needs and says to add only fields
a provider rejects.

## Out of scope

Falling through to the next target on a provider-specific 400. That is a failover policy change and needs its
own change and an ADR; this change removes the known cause instead.
