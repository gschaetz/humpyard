# switchyard-conductor

A unified Rust LLM gateway: cost/budget tracking, NVIDIA NeMo Switchyard intelligent routing,
and modelrelay-style multi-provider dispatch in one binary.

Status: pre-alpha. Today it is a protocol-translating proxy to one OpenAI-compatible upstream.
Routing, multiple providers and budgets are next. See [docs/background.md](docs/background.md).

See [docs/architecture.md](docs/architecture.md) for diagrams and component status and
[docs/routing.md](docs/routing.md) for routes, failover and the routing policy.

## Quickstart

```sh
cargo build --release
export GROQ_API_KEY=... OPENROUTER_API_KEY=...   # one key per provider in the config
./target/release/switchyard-conductor check-config examples/config.toml
./target/release/switchyard-conductor serve --config examples/config.toml
```

Clients can speak any of three protocols; providers are reached over OpenAI Chat Completions:

```sh
# OpenAI Chat Completions
curl localhost:8080/v1/chat/completions -H 'content-type: application/json' \
  -d '{"model":"fast","messages":[{"role":"user","content":"hi"}]}'

# Anthropic Messages (e.g. Claude Code: ANTHROPIC_BASE_URL=http://localhost:8080)
curl localhost:8080/v1/messages -H 'content-type: application/json' \
  -d '{"model":"fast","max_tokens":100,"messages":[{"role":"user","content":"hi"}]}'

# OpenAI Responses (e.g. Codex CLI)
curl localhost:8080/v1/responses -H 'content-type: application/json' \
  -d '{"model":"fast","input":"hi"}'
```

Also: `GET /v1/models`, `GET /healthz`. Set `"stream": true` for SSE.

## Configuration

See [examples/config.toml](examples/config.toml). Three kinds of entries:

- `[providers.<name>]`: an OpenAI-compatible endpoint. The API key is read from the environment
  variable named by `api_key_env`; an inline key is rejected. Optional `headers = { ... }` adds
  static HTTP headers to every call (authentication headers are rejected).
- `[targets.<name>]`: a model served by an ordered list of `{ provider, model }` endpoints.
  Clients may request a target by name; endpoints are tried in order, failing over on
  connection errors, timeouts, HTTP 429 and 5xx (not on other 4xx, and not once a stream has begun).
  Responses carry `x-conductor-target` and `x-conductor-provider` headers.
- `[routes.<name>]`: a built-in Switchyard algorithm over targets, requested by clients as the
  `model`: `passthrough`, `random` (weights, seed), `stage_router` (tool-result signals pick
  efficient vs capable), or `llm_classifier` (`capability` judges task difficulty, `escalation`
  latches to capable after repeated judge verdicts; both need a `judge` target). Send
  `x-switchyard-session-id` so per-session state (latches, holds) persists across requests.
  If the selected target fails entirely, the next target the algorithm returned is tried.

Logging is controlled by `RUST_LOG` (default `info`). `check-config` validates a file and reports
dangling provider/target references and missing key variables.

Not supported yet: budget/health-aware policy, `previous_response_id` (returns 400), inbound auth.

## Development

Specs live in `openspec/` (OpenSpec). `cargo test`, `cargo clippy --all-targets -- -D warnings`.
