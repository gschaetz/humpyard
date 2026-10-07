# switchyard-conductor

A unified Rust LLM gateway: cost/budget tracking, NVIDIA NeMo Switchyard intelligent routing,
and modelrelay-style multi-provider dispatch in one binary.

Status: pre-alpha. Today it is a protocol-translating proxy to one OpenAI-compatible upstream.
Routing, multiple providers and budgets are next. See [docs/background.md](docs/background.md).

## Quickstart

```sh
cargo build --release
export GROQ_API_KEY=...            # any OpenAI-compatible provider works
./target/release/switchyard-conductor check-config examples/config.toml
./target/release/switchyard-conductor serve --config examples/config.toml
```

Clients can speak any of three protocols; the upstream is always OpenAI Chat Completions:

```sh
# OpenAI Chat Completions
curl localhost:8080/v1/chat/completions -H 'content-type: application/json' \
  -d '{"model":"llama-3.3-70b-versatile","messages":[{"role":"user","content":"hi"}]}'

# Anthropic Messages (e.g. Claude Code: ANTHROPIC_BASE_URL=http://localhost:8080)
curl localhost:8080/v1/messages -H 'content-type: application/json' \
  -d '{"model":"llama-3.3-70b-versatile","max_tokens":100,"messages":[{"role":"user","content":"hi"}]}'

# OpenAI Responses (e.g. Codex CLI)
curl localhost:8080/v1/responses -H 'content-type: application/json' \
  -d '{"model":"llama-3.3-70b-versatile","input":"hi"}'
```

Also: `GET /v1/models`, `GET /healthz`. Set `"stream": true` for SSE.

## Configuration

See [examples/config.toml](examples/config.toml). `models` lists the names clients may request;
they are sent to the upstream unchanged. The API key is read from the environment variable named
by `api_key_env`; an inline key is rejected. Logging is controlled by `RUST_LOG` (default `info`).

Not supported yet: `previous_response_id` (returns 400), multiple upstreams, inbound auth.

## Development

Specs live in `openspec/` (OpenSpec). `cargo test`, `cargo clippy --all-targets -- -D warnings`.
