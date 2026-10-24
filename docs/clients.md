# Using humpyard with coding agents

## Telling the gateway what a request is

[Auto routing](routing.md#auto-routing-recipe) chooses a pool from the request's content (size, tools,
images, sub-agent) without any help from the client. What the gateway cannot guess is **intent**,
for example "this conversation is private". A client says so by sending a header:

| Header | Meaning |
|---|---|
| `x-humpyard-profile: <name>` | a free-form profile (for example `private`) that rules can match |
| `x-humpyard-tag-<name>: <value>` | free-form tags (for example `x-humpyard-tag-privacy: private`) |

Most agent harnesses send neither and cannot vary headers per request, but many let you set
**static headers on a provider**. Then add one provider entry per intent, all with the same URL and
key, and let the user pick the entry:

```json
"humpyard":         { "baseUrl": "https://gateway.example/v1", "api": "openai-completions", "apiKey": "...", "models": [{ "id": "auto", "name": "auto" }] },
"humpyard-private": { "baseUrl": "https://gateway.example/v1", "api": "openai-completions", "apiKey": "...",
                      "headers": { "x-humpyard-profile": "private" }, "models": [{ "id": "auto", "name": "auto-private" }] }
```

Both ask for `auto`; the second always carries the private profile, so the privacy rule fires.

- **OpenClaw:** per-provider `headers` are part of its provider schema (checked against its config
  schema, version 2026.9.5; the entry above is the shape).
- **Codex CLI:** custom providers accept `http_headers` in `config.toml` (seen in its source; not
  yet exercised against the gateway).
- **Claude Code:** I have not verified a way to add headers; use a separate gateway key per intent
  and match on `key` in a rule until that is checked.
- A client that sends **no** identity (the OpenAI JS SDK, used by some harnesses, sends only a
  generic user-agent) can still be distinguished by **key**: one gateway key per consumer, matched
  with `when = { key = "..." }`.

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
- **A multi-step task with a subagent and a resumed session (verified 2026-10-08).** In a scratch
  Python project Claude Code fixed a bug, added a method and tests, ran the suite, wrote docs,
  delegated a review to a subagent, then (resuming the same session with `--continue`, which
  replays the whole history including thinking blocks) added another feature: 17 requests in total,
  all HTTP 200, 10/10 tests passing, one escalation to the capable tier. Unlike Codex, the
  Anthropic path needed no request adaptation. Claude Code's cumulative session usage matched the
  ledger exactly (121,385 input, 189,696 cached, 5,595 output).
- **Daily use.** Run the gateway container and a wrapper that sets `CLAUDE_CONFIG_DIR` (so your
  normal Claude Code setup is untouched), `ANTHROPIC_BASE_URL`, a gateway key in
  `ANTHROPIC_API_KEY` and the model variables above, then `exec claude "$@"`.
- **`POST /v1/messages/count_tokens` is answered with a local estimate.** No provider has a count
  call, so the number is computed by the gateway (no upstream call, no cost, no ledger entry, still
  available to a key over budget) and marked with `x-humpyard-token-count: estimate`. Measured
  against two real providers it ran 1.07x to 2.14x the true prompt size (median 1.23x), never below
  it: clients compact slightly early rather than overflow. Don't use it for billing.

## OpenClaw (verified 2026-10-08, openclaw 2026.9.5)

OpenClaw talks to any OpenAI-compatible endpoint through `models.providers` in its config
(`api = "openai-completions"`). Declare the gateway as a provider and list your route names as
models:

```json
{
  "models": { "providers": { "humpyard": {
    "baseUrl": "http://127.0.0.1:8080/v1",
    "api": "openai-completions",
    "apiKey": "sk-humpyard-...",
    "models": [ { "id": "agent", "name": "agent", "contextWindow": 128000, "input": ["text"] } ]
  } } },
  "agents": { "defaults": { "model": { "primary": "humpyard/agent" } } }
}
```

`openclaw agent --local --model humpyard/agent --message "..."` then runs an embedded agent turn
through the gateway. What was verified, with real paid models behind the routes:

- **Heavy requests work.** Each turn carries 50+ tool definitions and prompts of 30,000 to 70,000
  tokens, streamed with `include_usage`; they translate and stream back correctly.
- **A tool round trip works.** The agent called its `exec` tool, received the result and answered
  correctly over two turns.
- **It sends no identity by default.** The requests carry the OpenAI JS SDK's user-agent and nothing
  else: no session id, agent id or metadata. Setting `compat.sendSessionAffinityHeaders: true` on a model
  makes it send `session_id` and `x-session-affinity`, which humpyard reads as the session id (from 0.8.5),
  so its agents appear in `group_by=session` and the switch report. Tell consumers apart with one gateway key each (and
  `allowed_routes`); the provider entry also accepts `headers`, so `x-humpyard-profile` or
  `x-humpyard-tag-*` can be added if rules should depend on them.
- **Plan prompt-size thresholds around its size.** Even "reply with pong" is a 30,000-token
  prompt, so a `prompt_tokens` rule or a model with a small context window will see every request
  as large.

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
- **A strict provider in the pool broke every request (found 2026-10-23, codex-cli 0.163.0).** Codex sends
  `store` and `prompt_cache_key`; Google AI rejects both with a 400, which does not fail over, so a route whose
  first usable target was Google AI failed every time while the same Codex run on another route passed. The
  provider's `drop_fields` setting removes them; see
  [Providers](providers.md#providers-that-reject-unknown-request-fields). A 400 or 422 now moves a pool on to its
  next member (class `rejected`, at most three different providers, never counted against the endpoint's health),
  so one strict member costs a round trip instead of the whole route; `drop_fields` still removes that round trip. Codex 0.163.0 also sends 12 tools
  including `multi_agent_v1__*` and the goal tools; Gemini accepted them all once those two fields were gone.
- **Gemini's second turn needs a thought signature (found 2026-10-23).** With the first request fixed, the next
  turn failed: Gemini 3.x wants `extra_content.google.thought_signature` on the replayed function call. The
  provider setting `tool_call_extra_content` supplies Google's documented placeholder; see
  [Providers](providers.md#gemini-3x-wants-a-thought-signature-on-replayed-tool-calls).
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
