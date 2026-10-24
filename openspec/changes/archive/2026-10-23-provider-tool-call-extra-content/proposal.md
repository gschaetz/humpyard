# Proposal

## Why

With `drop_fields` live (0.14.0), Codex's first request on `deep-thinker-public` is served by Gemini 3.6 Flash, but
its second turn fails: `Function call is missing a thought_signature in functionCall parts` (HTTP 400, which does
not fail over). Gemini 3.x returns a signature in `tool_calls[].extra_content.google.thought_signature`, and wants
it back on the replayed assistant tool call. Checked directly against Gemini with a captured Codex request:

| second turn sends | result |
|---|---|
| no `extra_content` | 400, as in production |
| the real signature | 200 |
| `skip_thought_signature_validator` | 200 |

The signature cannot be carried through the gateway today: the Switchyard translation has no place for provider
data on a tool call (`ToolCall` is `id`, `name`, `arguments`), so the real value is dropped when the response is
decoded, and a Responses client such as Codex has nowhere to hold it either. Google documents the
`skip_thought_signature_validator` value for exactly this case, a conversation whose earlier function calls did not
come from Gemini (quality may degrade a little).

## What Changes

A provider MAY set `tool_call_extra_content`, a table. The gateway adds it as `extra_content` to every assistant
tool call that is replayed to that provider. For Google AI it is
`{ google = { thought_signature = "skip_thought_signature_validator" } }`. Providers without the setting, and
requests without replayed tool calls, are untouched.

## Capabilities

### New Capabilities

### Modified Capabilities
- `upstream-proxy`: replayed tool calls get a provider's configured `extra_content`.
- `gateway-config`: the provider `tool_call_extra_content` setting.

## Impact

`src/config/{schema,mod,validate}.rs`, `src/pool.rs`, new `tests/tool_call_extra_content.rs`, docs/configuration.md,
docs/providers.md, docs/clients.md. The cluster configuration needs the setting on its Google AI provider.
