# Tasks

## 1. Project setup

- [x] 1.1 Add `rust-toolchain.toml`, dependencies (axum, tokio, reqwest, clap, serde, toml, tracing, switchyard-protocol, switchyard-translation) and verify `cargo build` succeeds
- [x] 1.2 Add CI workflow running `cargo fmt --check`, `clippy -D warnings`, `cargo test` and verify it passes on a pushed branch

## 2. Configuration and CLI

- [x] 2.1 Implement TOML config structs with `deny_unknown_fields`, env-var key loading and inline-key rejection; unit tests cover valid, unknown-key, missing-var cases
- [x] 2.2 Implement `serve` and `check-config` subcommands; verify `check-config` exits 0/non-zero as specced via integration test

## 3. Translation and error layer

- [x] 3.1 Add a `translate` module wrapping decode/encode for OpenAI Chat, OpenAI Responses and Anthropic Messages; fixture tests cover OpenAI Responses, plain, tool-call and streaming payloads
- [x] 3.2 Implement the error enum with per-endpoint rendering; tests cover 400, 404, 429, 502, 504 shapes

## 4. Gateway endpoints

- [x] 4.1 Implement `/healthz` and `/v1/models`; tests verify responses
- [x] 4.2 Implement non-streaming `/v1/chat/completions` against a mock upstream; test asserts bearer header and response shape
- [x] 4.3 Implement streaming path with incremental forwarding and `[DONE]`; test asserts first chunk arrives before upstream completes
- [x] 4.4 Implement `/v1/messages` (stream and non-stream) over the same pipeline; tests assert Anthropic-format output from an OpenAI upstream
- [x] 4.5 Implement `/v1/responses` (stream and non-stream, function tools) over the same pipeline; tests assert Responses SSE event order and `function_call` output from an OpenAI Chat upstream, and a 400 for `previous_response_id`
- [x] 4.6 Cancel upstream on client disconnect; test asserts mock upstream observes the drop

## 5. Logging and docs

- [x] 5.1 Add `tracing` request logs with key redaction; test asserts the key never appears in captured logs
- [x] 5.2 Document config and a curl/Claude Code quickstart in README.md and verify the commands work against a local run
