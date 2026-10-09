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

## Codex CLI (verified 2026-10-08, codex-cli 0.161.0, `brew install --cask codex`)

Codex speaks the OpenAI Responses protocol, which humpyard serves at `/v1/responses`. Point it at
the gateway with a custom provider; the flags below keep the run isolated from any real Codex setup:

```sh
export HUMPYARD_KEY=sk-humpyard-...          # a key from `humpyard keygen`
codex exec --ignore-user-config --ephemeral --skip-git-repo-check -s read-only \
  -c 'model_provider="humpyard"' \
  -c 'model_providers.humpyard={name="humpyard", base_url="http://127.0.0.1:8080/v1", env_key="HUMPYARD_KEY", wire_api="responses"}' \
  -m agent "your task"
```

(For regular use put the same provider in `~/.codex/config.toml` under `[model_providers.humpyard]`
and set `model_provider = "humpyard"`.) `agent` is a route name from the gateway config.

What was verified, with a real model behind the route:

- **Shell tool calls round trip.** Codex ran `cat src/lib.rs` and used the output.
- **File edits work.** Codex's freeform `apply_patch` tool, which the Responses translation maps to
  and from an ordinary function call, produced a correct patch: `mul` was added to `src/lib.rs`.
- **Streaming, reasoning items and prompt caching** pass through. Cached prompt tokens are
  reported and priced.
- **Accounting matches Codex exactly.** Over a three-request edit task Codex reported 27,241 input
  tokens (14,592 cached) and 314 output tokens; the ledger rows sum to the same figures. All
  upstream requests were HTTP 200.
- **Cosmetic:** Codex prints "Model metadata for `agent` not found" for non-OpenAI model names.
- **Longer sessions need request adaptation (found 2026-10-08).** Short runs passed, but the first
  real multi-step coding task failed with upstream 400s once the conversation had history: Codex
  replays its earlier reasoning items and injects `developer`-role messages, neither of which chat
  providers accept. The pool now drops Responses-shaped reasoning details and sends `developer` as
  `system` (change `adapt-requests-for-chat-providers`); with that, Codex found and fixed two bugs
  in a small Python project through the gateway (6 requests, all HTTP 200).
- **Model limits.** Codex has no metadata for non-OpenAI model names; give it the limits in its
  config (`model_context_window`, `model_auto_compact_token_limit`).

### Daily use with a local container

Run the gateway container (see [deployment.md](deployment.md)) and give Codex its own `CODEX_HOME`
so your normal setup is untouched:

```toml
# ~/.codex-humpyard/config.toml
model_provider = "humpyard"
model = "agent"                       # a route name from the gateway config
model_context_window = 128000
model_auto_compact_token_limit = 100000

[model_providers.humpyard]
name = "humpyard"
base_url = "http://127.0.0.1:8080/v1"
env_key = "HUMPYARD_KEY"
wire_api = "responses"
```

```sh
#!/bin/sh   # ~/.local/bin/codex-hy
HUMPYARD_KEY=$(cat "$HOME/.config/humpyard/codex-key") || exit 1
export HUMPYARD_KEY CODEX_HOME="$HOME/.codex-humpyard"
exec codex "$@"
```

- **Token counting and compaction.** `POST /v1/responses/input_tokens` is answered with the same
  local estimate as `count_tokens` (`{"object": "response.input_tokens", "input_tokens": n}`).
  `POST /v1/responses/compact` is deliberately **not** implemented: it returns OpenAI-specific
  encrypted compaction items, and Codex does not call it for custom providers. Their remote
  compaction capability defaults to unsupported (only OpenAI and Azure providers get it), so Codex
  compacts locally by summarizing through ordinary `/v1/responses` calls, which work.
