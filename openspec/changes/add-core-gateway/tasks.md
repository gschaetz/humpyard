# Tasks

## 1. Project setup

- [ ] 1.1 Add `rust-toolchain.toml`, dependencies (axum, tokio, reqwest, clap, serde, toml, tracing, switchyard-protocol, switchyard-translation) and verify `cargo build` succeeds
- [ ] 1.2 Add CI workflow running `cargo fmt --check`, `clippy -D warnings`, `cargo test` and verify it passes on a pushed branch

## 2. Configuration and CLI

- [ ] 2.1 Implement TOML config structs with `deny_unknown_fields`, env-var key loading and inline-key rejection; unit tests cover valid, unknown-key, missing-var cases
- [ ] 2.2 Implement `serve` and `check-config` subcommands; verify `check-config` exits 0/non-zero as specced via integration test

## 3. Translation and error layer

- [ ] 3.1 Add a `translate` module wrapping decode/encode for OpenAI Chat and Anthropic Messages; fixture tests cover plain, tool-call and streaming payloads
- [ ] 3.2 Implement the error enum with per-endpoint rendering; tests cover 400, 404, 429, 502, 504 shapes

## 4. Gateway endpoints

- [ ] 4.1 Implement `/healthz` and `/v1/models`; tests verify responses
- [ ] 4.2 Implement non-streaming `/v1/chat/completions` against a mock upstream; test asserts bearer header and response shape
- [ ] 4.3 Implement streaming path with incremental forwarding and `[DONE]`; test asserts first chunk arrives before upstream completes
- [ ] 4.4 Implement `/v1/messages` (stream and non-stream) over the same pipeline; tests assert Anthropic-format output from an OpenAI upstream
- [ ] 4.5 Cancel upstream on client disconnect; test asserts mock upstream observes the drop

## 5. Logging and docs

- [ ] 5.1 Add `tracing` request logs with key redaction; test asserts the key never appears in captured logs
- [ ] 5.2 Document config and a curl/Claude Code quickstart in README.md and verify the commands work against a local run
