# 0002. Dispatch on Switchyard's client; providers speak OpenAI Chat

- Status: accepted
- Date: 2026-10-07 (recorded retroactively; decided 2026-10-07)
- Supersedes: none
- Change: [add-switchyard-routing](../../openspec/changes/archive/2026-10-06-add-switchyard-routing), [add-provider-headers](../../openspec/changes/archive/2026-10-06-add-provider-headers)

## Context

The first version proxied to one upstream with a hand-written `reqwest` relay: SSE parsing, error
mapping, timeouts. Switchyard's `TranslatingLlmClient` already provides per-model HTTP backends,
retries with backoff, timeouts, streaming and translation, and `run` already tries a route's
selected targets in order. A spike confirmed `run` streams the final answer and keeps per-session
state inside the algorithm instances.

## Decision

All upstream calls go through Switchyard's `run` and `TranslatingLlmClient`. Each configured
**target** is one `RoutedLlmClient` (our `TargetClient`) that walks the target's ordered endpoints,
using one `TranslatingLlmClient` per endpoint so each endpoint keeps its own model name, key and
headers. Failover happens inside the target on connection errors, timeouts, HTTP 408, 429 and 5xx,
never on other 4xx and never after a stream has started. Providers are reached over OpenAI Chat
Completions only; client protocols (Chat, Responses, Anthropic) are translated at the edge.

Alternatives considered: keep our relay (more code, own streaming edge cases); one Switchyard model
id per endpoint (skews `random` weights and stage-router tiers because targets multiply).

## Consequences

- A fraction of the code and no SSE parsing of our own; Anthropic and Responses *backends* become
  configuration later, not new code.
- Each endpoint exhausts its provider's `max_retries` before failover moves on, so retries and
  failover interact (documented in `docs/routing.md`).
- Switchyard's client only supports static headers per provider, so a per-conversation provider
  header (OpenCode Go's `x-opencode-session`) cannot be sent yet (`docs/providers.md`).
- Some models need a non-OpenAI protocol (`minimax-m2.7`) and are unsupported until Anthropic
  backends are configurable.
