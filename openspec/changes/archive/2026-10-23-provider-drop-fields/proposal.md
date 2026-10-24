# Proposal

## Why

Codex CLI 0.163.0 through the live gateway failed every request on `deep-thinker-public` with
`upstream returned 400 Bad Request` (2026-10-23). The route's first usable target was Google AI, whose
OpenAI-compatible endpoint rejects fields it does not know. The translation copies the Responses
request's `prompt_cache_key` and `store` into the chat body, and Google answers
`Unknown name "prompt_cache_key": Cannot find field.` (and the same for `store`). Replaying the captured
request directly confirmed both fields are needed to fail: with both removed Google returns 200 and a tool
call; with either one left the request still fails.

Two things made this costly:
1. The 400 is a client error, so the route stops instead of trying the next target. One strict provider
   therefore takes down the whole route for a client that sends those fields.
2. Google wraps its error JSON in an array (`[{"error": {...}}]`), which the error mapping reduces to a bare
   `upstream returned 400`, so the field name never reached the client or the logs.

Other providers accept `prompt_cache_key` and use it for cache routing, and the ledger prices cached tokens,
so stripping it for every provider would lose real savings.

## What Changes

- A provider MAY list `drop_fields`: top-level request fields that are removed from every request sent to that
  provider. Only the translation's pass-through extension fields are affected; the field is also removed from
  a preserved same-format request body, so a chat client that sends it is covered too.
- An upstream error whose JSON body is an array holding the provider's error object now yields the provider's
  message, like a bare object does.

## Capabilities

### New Capabilities

### Modified Capabilities
- `upstream-proxy`: per-provider removal of request fields; array-shaped provider errors are read.
- `gateway-config`: the provider `drop_fields` setting.

## Impact

`src/config/{schema,mod,validate}.rs`, `src/pool.rs`, `src/error.rs`, new `tests/drop_fields.rs`,
docs/configuration.md, docs/providers.md, docs/clients.md. The cluster configuration needs
`drop_fields` on its Google AI provider after a release carries this.
