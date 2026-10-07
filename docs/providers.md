# Provider notes

Findings from running the gateway against real providers. Mock tests cover behavior; these notes
cover what real services need.

## OpenCode Go (verified 2026-10-07)

Base URL `https://opencode.ai/zen/go/v1`, OpenAI Chat Completions, bearer key.

```toml
[providers.opencode]
base_url = "https://opencode.ai/zen/go/v1"
api_key_env = "OPENCODE_API_KEY"
headers = { "x-opencode-session" = "switchyard-conductor", "user-agent" = "switchyard-conductor/0.1" }
```

- **`x-opencode-session` is required** (HTTP 400 without it). The provider wants a stable id per
  conversation for routing and prompt caching; the gateway can only send a static value today
  (`headers`), so all conversations share one id. A per-conversation value is a follow-up.
- **Some models depend on account settings.** `deepseek-v4-*` returned "requires Global regions"
  until the workspace privacy setting is changed. `qwen3.5-plus` was unavailable.
- **Some models need another protocol.** `minimax-m2.7` answers "does not support this protocol"
  on the OpenAI endpoint (it needs an Anthropic-format backend, which the gateway does not
  configure yet).
- Models that worked: `glm-5.3-flash`, `glm-5.2`, `kimi-k2.6`, and the free
  `longcat-2.5-preview-free`.
- **Reasoning models spend `max_tokens` on reasoning.** With a tiny `max_tokens` the visible
  content can be empty; use a few hundred tokens in tests.

## What was verified end to end

Against real models, with a dead first endpoint to force failover:

- Chat Completions, Anthropic Messages and Responses, buffered and streaming (event shapes
  complete for all three).
- Failover from a refused connection to the working endpoint on every call, with
  `x-conductor-target` and `x-conductor-provider` set.
- A tool call and a tool-result round trip.
- `stage_router`: a failing tool result escalated to the capable target; clean turns stayed on
  the efficient one. A clean test pass clears the capable hold early (Switchyard behavior).
- `llm_classifier` in `capability` mode with a real judge: an easy question stayed on the efficient
  target, a hard one went to the capable target.

## Not yet verified

Rate-limit (429) failover against a real provider, Responses `previous_response_id`, and
long-running streams near the provider timeout.
