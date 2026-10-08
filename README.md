# humpyard

A gateway that sorts each LLM request onto the right model, in one Rust binary: OpenAI Chat,
OpenAI Responses and Anthropic Messages in; routing by NVIDIA NeMo Switchyard's algorithms;
multi-provider failover; virtual keys, a usage ledger and budgets.

*Why the name:* a hump yard is the railroad yard where cars are classified and sent down the right
track, one by one. That is the job here, with requests and models. Home: https://humpyard.dev.
(Formerly `switchyard-conductor`.)

Status: pre-alpha. Working today: the three client protocols with streaming, routes backed by
Switchyard's `passthrough`, `random`, `stage_router` and `llm_classifier`, ordered-endpoint
failover with per-endpoint circuit breakers, a routing-policy seam, virtual keys, a SQLite usage
ledger and per-key budgets. Planned: health-driven target eligibility, database-managed keys, the
modelrelay migration command. Design
background in [docs/background.md](docs/background.md).

Run it: release binaries, a container image and a macOS launchd service are described in
[docs/deployment.md](docs/deployment.md).

See [docs/architecture.md](docs/architecture.md) for diagrams and component status and
[docs/routing.md](docs/routing.md) for routes, failover and the routing policy, and
[docs/budgets.md](docs/budgets.md) for virtual keys, the usage ledger and budgets.

## Quickstart

```sh
cargo build --release
export GROQ_API_KEY=... OPENROUTER_API_KEY=...   # one key per provider in the config
./target/release/humpyard check-config examples/config.toml
./target/release/humpyard serve --config examples/config.toml
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

Also: `GET /v1/models`, `GET /healthz`, `GET /v1/key/info`, and `POST /v1/messages/count_tokens`
and `POST /v1/responses/input_tokens` (conservative local estimates: no provider call, no cost). Verified with real Claude Code and
Codex CLI sessions: see [docs/clients.md](docs/clients.md). Set `"stream": true` for SSE.

## Configuration

See [examples/config.toml](examples/config.toml). Three kinds of entries:

- `[providers.<name>]`: an OpenAI-compatible endpoint. The API key is read from the environment
  variable named by `api_key_env`; an inline key is rejected. Optional `headers = { ... }` adds
  static HTTP headers to every call (authentication headers are rejected).
- `[targets.<name>]`: a model served by an ordered list of `{ provider, model }` endpoints.
  Clients may request a target by name; endpoints are tried in order, failing over on
  connection errors, timeouts, HTTP 429 and 5xx (not on other 4xx, and not once a stream has begun).
  Responses carry `x-humpyard-target` and `x-humpyard-provider` headers.
- `[routes.<name>]`: a built-in Switchyard algorithm over targets, requested by clients as the
  `model`: `passthrough`, `random` (weights, seed), `stage_router` (tool-result signals pick
  efficient vs capable), or `llm_classifier` (`capability` judges task difficulty, `escalation`
  latches to capable after repeated judge verdicts; both need a `judge` target). Send
  `x-switchyard-session-id` so per-session state (latches, holds) persists across requests.
  If the selected target fails entirely, the next target the algorithm returned is tried.
- `[health]`: optional endpoint circuit breaker (`failure_threshold`, `cooldown_secs`,
  `max_cooldown_secs`); `GET /v1/health` shows which endpoints are being skipped.

Logging is controlled by `RUST_LOG` (default `info`).

**Shutdown.** SIGTERM or Ctrl-C starts a graceful stop: new connections are refused, requests
already running may finish for up to `shutdown_grace_secs` (default 30; a second signal stops
waiting at once), anything still running is then cancelled (streams end with an error event), and
queued usage entries are written to the ledger before the process exits 0. A kill -9 or crash can
still lose entries that were queued but not yet written. `check-config` validates a file and reports
dangling provider/target references and missing key variables.

Not supported yet: health-driven target eligibility, database-managed keys, `previous_response_id` (returns 400).

## Development

Specs live in `openspec/` (OpenSpec, with an ADR step); decisions in [docs/adr/](docs/adr/); the
rules that must keep holding, and what enforces them, in [docs/invariants.md](docs/invariants.md).
Locally: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`; CI also
runs `cargo deny check`, a warning-free `cargo doc`, an MSRV build and a coverage floor.

## License

Apache-2.0, see [LICENSE](LICENSE) and [NOTICE](NOTICE). Contributions require a DCO sign-off,
see [CONTRIBUTING.md](CONTRIBUTING.md).
