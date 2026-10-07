# switchyard-conductor

A unified Rust LLM gateway: cost/budget tracking, NVIDIA NeMo Switchyard intelligent routing,
and modelrelay-style multi-provider dispatch in one binary.

Status: pre-alpha. Today it is a protocol-translating proxy to one OpenAI-compatible upstream.
Routing, multiple providers and budgets are next. See [docs/background.md](docs/background.md).

See [docs/architecture.md](docs/architecture.md) for diagrams and component status.

## Quickstart

```sh
cargo build --release
export GROQ_API_KEY=... OPENROUTER_API_KEY=...   # one key per provider in the config
./target/release/switchyard-conductor check-config examples/config.toml
./target/release/switchyard-conductor serve --config examples/config.toml
```

Clients can speak any of three protocols; the upstream is always OpenAI Chat Completions:

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
  variable named by `api_key_env`; an inline key is rejected.
- `[targets.<name>]`: a model served by an ordered list of `{ provider, model }` endpoints.
  Clients may request a target by name today (served by its first endpoint).
- `[routes.<name>]`: a built-in Switchyard algorithm (`passthrough`, `random`, `stage_router`,
  `llm_classifier`) over targets. Parsed and validated now; served once routing lands
  (`add-switchyard-routing` change).

Logging is controlled by `RUST_LOG` (default `info`). `check-config` validates a file and reports
dangling provider/target references and missing key variables.

Not supported yet: routing, endpoint failover, `previous_response_id` (returns 400), inbound auth.

## Development

Specs live in `openspec/` (OpenSpec). `cargo test`, `cargo clippy --all-targets -- -D warnings`.
