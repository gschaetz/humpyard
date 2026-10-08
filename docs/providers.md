# Provider notes

Findings from running the gateway against real providers. Mock tests cover behavior; these notes
cover what real services need.

## OpenCode Go (verified 2026-10-07)

Base URL `https://opencode.ai/zen/go/v1`, OpenAI Chat Completions, bearer key.

```toml
[providers.opencode]
base_url = "https://opencode.ai/zen/go/v1"
api_key_env = "OPENCODE_API_KEY"
headers = { "x-opencode-session" = "humpyard", "user-agent" = "humpyard/0.1" }
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
  `x-humpyard-target` and `x-humpyard-provider` set.
- A tool call and a tool-result round trip.
- `stage_router`: a failing tool result escalated to the capable target; clean turns stayed on
  the efficient one. A clean test pass clears the capable hold early (Switchyard behavior).
- `llm_classifier` in `capability` mode with a real judge: an easy question stayed on the efficient
  target, a hard one went to the capable target.

## Budgets, usage and shutdown against a real provider (verified 2026-10-08)

Same provider (OpenCode Go; `glm-5.3-flash` as the efficient tier, `kimi-k2.6` as the capable tier,
the free `longcat-2.5-preview-free` as the free tier), a throwaway gateway key with a 1,500-token
daily limit and another with a 400-token `free_only` limit, at 50% restricted.

- **Usage matches the provider exactly.** Buffered, streamed (the final usage chunk the client sees
  equals the ledger row), Anthropic-format and Responses-format calls all recorded the provider's
  prompt and completion tokens, and the key's running total matched the sum of the calls (738
  tokens after five calls). The first run exposed a real bug: reasoning tokens (a subset of the
  completion tokens) were being added again, doubling reasoning-heavy calls; fixed in PR #32.
- **Cost is right.** A `kimi-k2.6` call of 53 input and 120 output tokens at $1 and $4 per million
  came to 533 micro-USD; cached-prompt tokens bill at the input price unless `cached_input` is set.
- **States progress as designed.** Healthy: a failing tool turn escalated to the capable tier.
  Past 50% (restricted): identical failing turns were served by the efficient tier, because the
  capable tier's price is above the ceiling. Past the limit: HTTP 402 in the chat, Anthropic and
  Responses shapes, with no upstream call (the ledger row count did not move). The last call
  crossed the limit (1,633 of 1,500 tokens): the documented overshoot.
- **Free-only keys continue on free targets**, and a route with no free target gets 402.
- **Graceful shutdown in the middle of a real stream.** After SIGTERM new connections were
  refused, the running stream finished (192 events plus `[DONE]`), the gateway exited after it
  completed, and the ledger row had full usage with outcome `ok`.
- **Restart recovery.** After a restart both keys' spend was rebuilt from the ledger (1,633 and 910
  tokens) and the exhausted key still got 402.

## Failover against real network failures (verified 2026-10-08)

A target whose first endpoint fails for real, ahead of a working OpenCode Go endpoint:

| first endpoint | result | time |
|---|---|---|
| connect timeout (non-routable address, 3 s limit) | failed over, served by OpenCode | 3.6 s |
| DNS failure (`.invalid` name) | failed over, served by OpenCode | 0.8 s |
| a real server answering **404** (public test service) | **no failover**: the 404 passed straight to the client | 0.2 s |
| the only endpoint times out | client got 504 `upstream timed out` | 3.0 s |

- The log names each failed endpoint and why ("trying next ... provider=blackhole ... timed out").
- The ledger holds only the call that served, attributed to the serving endpoint; failed attempts
  carry no usage and are not recorded.
- A real 404 not failing over is the intended rule (other 4xx would fail identically elsewhere),
  now seen against a real server, not just a mock.
- **Real 429 and 5xx responses could not be produced safely** (provoking OpenCode Go's own limits
  would burn quota, and public status services answer only their exact paths, while the gateway
  appends `/chat/completions`). 429/5xx failover is covered by the mock-provider tests in
  `tests/pool.rs`.
- Observed rough edge: when an upstream returns a non-JSON body (an HTML 404 page), the client's
  error message and the log carry that raw text. A status-based message would be cleaner.

**Safety rule when testing failover:** the gateway sends each provider's API key to that
provider's URL. Give any endpoint you do not control (a public test service, a black hole) its own
dummy `api_key_env`, never the key of a real provider.

## Grace-period cut with a real stream (verified 2026-10-08)

A long streamed answer from OpenCode Go (glm-5.3-flash), `shutdown_grace_secs = 2`, SIGTERM sent
6 s in (about 36 KB already delivered):

- The log shows "shutdown requested; draining", then "grace period over; cancelling", exactly 2 s
  later; the gateway exited cleanly right after.
- The client's stream ended 2.0 s after the signal with an error event
  (`upstream unreachable: gateway is shutting down`) followed by `data: [DONE]`, not a dropped
  connection.
- The ledger recorded the request with outcome `cancelled`.
- **Found:** that row had zero tokens and zero cost, because a cut stream never reaches the
  provider's final usage chunk, although the provider generated (and may bill) the delivered
  text. Fixed by `estimate-unreported-stream-usage`: such rows now carry an output-only estimate
  (marked `usage_missing`); the input side of a cut stream is still not counted.

## Not yet verified

A real provider's own 429 or 5xx, Responses `previous_response_id`, and long-running streams near the
provider timeout.
