# Proposal

## Why

Some models need request settings that clients do not send. On the operator's own `friday`, `qwen3.5:35b-a3b` thinks by
default: a bare coding prompt ran to the 8,000-token limit and came back empty after 109 seconds. The same prompt with
`reasoning_effort: "none"` answered in 39 tokens, and with the model's instruct sampling (`temperature 0.7`, `top_p 0.8`)
a fixed task set went from 79 of 87 passed to 85 of 87. Only a client that sends those fields gets that. Claude Code,
Codex and most agents do not, and the gateway cannot pass on what the client never said. The setting belongs to the
model, so the operator who knows the model should be able to state it once, where the model is configured.

## What Changes

- An endpoint (a provider and a model in a target) accepts `request_defaults`: a table of request fields the gateway adds
  to a request sent to that endpoint **only when the request does not already carry the field**. A client that sends its
  own `temperature` or `reasoning_effort` keeps it.
- The allowed fields are `reasoning_effort`, `temperature`, `top_p`, `top_k`, `min_p`, `presence_penalty` and
  `frequency_penalty`, each checked for type and range when the configuration loads. Any other name, or a wrong type or
  range, is refused at load and by `check-config`, naming the endpoint and the field.
- The defaults are applied to the request as the endpoint receives it, after the request has been translated to the
  endpoint's protocol, so a request that arrived in the Anthropic format gets the same defaults as one that arrived in the
  OpenAI format.
- The effective configuration (`GET /v1/config`, the admin configuration view) shows each endpoint's `request_defaults`.
- Documentation: the configuration guide says what the setting is for, that a client's own value wins, and what a
  Claude Code request can and cannot express (see design).
- **Out of this change:** a way to force a value over the client's (`request_overrides`), defaults on a route or a key,
  `max_tokens` or any field that changes budget or cost estimation, recording on each decision which defaults were applied
  (the HTTP client merges them internally and does not say which it filled), and any change to the Modelfile on `friday`.

## Capabilities

### New Capabilities

None.

### Modified Capabilities
- `gateway-config`: an endpoint may declare `request_defaults`, which are validated at load and shown in the effective
  configuration.
- `upstream-proxy`: a request to an endpoint carries the endpoint's declared defaults for the fields it lacks, and never
  replaces a field it has.

## Impact

- **Code:** one field on `Endpoint` in `config/schema.rs` (the effective configuration serializes the endpoint, so it
  shows there too) with its validation in `config/validate.rs`; passing the table as the backend's extra body in `pool.rs`
  (`endpoint_client`, where it is an empty map today). No new module, no change to the request path's order, no new work per request beyond the merge the HTTP client already
  performs.
- **Dependencies:** none. The `switchyard-llm-client` crate already merges an `extra_body` map into an outbound body
  without replacing the caller's fields; humpyard sets it to empty today.
- **Compatibility:** an endpoint without the setting behaves exactly as before. The setting is refused by an older
  gateway (unknown field), so a configuration that uses it must not be deployed to an older version.
