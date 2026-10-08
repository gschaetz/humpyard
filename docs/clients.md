# Using humpyard with coding agents

## Claude Code (verified 2026-10-08, Claude Code 2.1.285)

Claude Code speaks the Anthropic Messages protocol, which humpyard accepts at `/v1/messages`
(streaming, tool use, parallel tool calls and tool errors all pass through).

```sh
export ANTHROPIC_BASE_URL=http://127.0.0.1:8080        # where `humpyard serve` listens
export ANTHROPIC_API_KEY=sk-humpyard-...                # a key from `humpyard keygen`
export ANTHROPIC_MODEL=agent                            # a route name from your config
export ANTHROPIC_DEFAULT_SONNET_MODEL=agent
export ANTHROPIC_DEFAULT_OPUS_MODEL=agent
export ANTHROPIC_DEFAULT_HAIKU_MODEL=fast               # small, cheap calls (titles, summaries)
export ANTHROPIC_SMALL_FAST_MODEL=fast                  # older name for the same slot
claude
```

A route that suits it, escalating to the capable model after tool failures:

```toml
[routes.agent]
type = "stage_router"
efficient = ["fast"]
capable = ["smart"]
```

Notes from the verified run (a read-only agent in a scratch directory, `claude --bare`):

- **Don't name a route `auto`.** Claude Code has its own "auto mode"; the name triggered a client
  message about it. `agent` (or anything else) is fine.
- **The `unrecognized_model` notice** Claude Code prints for non-Claude model names is cosmetic.
- **Routing in action.** The stage router started on the efficient model, moved to the capable one
  after the first tool errors, and every call, with its cached-prompt tokens, was recorded in the
  ledger under the key (14 calls, all HTTP 200).
- **Model quality matters more than the gateway here.** On an open-ended search task a small model
  guessed 39 different filenames (`todo.txt`, `main.py`, ...) without using the search tools and ran
  out of turns, while the same agent answered a precise "read this file" task correctly in two
  turns. Pick tiers for agent work accordingly.
- **`POST /v1/messages/count_tokens` is answered with a local estimate.** No provider has a count
  call, so the number is computed by the gateway (no upstream call, no cost, no ledger entry, still
  available to a key over budget) and marked with `x-humpyard-token-count: estimate`. Measured
  against two real providers it ran 1.07x to 2.14x the true prompt size (median 1.23x), never below
  it: clients compact slightly early rather than overflow. Don't use it for billing.

## Codex and other Responses-API clients

`/v1/responses` is implemented and tested against mock providers and with curl against a real one,
but a real Codex CLI session has not been run yet.
