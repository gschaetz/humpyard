# Tasks

## 1. The setting and its checks

- [x] 1.1 Add `request_defaults` (an optional table, omitted when empty) to `Endpoint` in `config/schema.rs`, and validate it in `config/validate.rs` through a function that takes the endpoint's protocol (OpenAI chat is the only one), with the target name and endpoint position in the message: only `reasoning_effort`, `temperature`, `top_p`, `top_k`, `min_p`, `presence_penalty` and `frequency_penalty`, each with its type and range; verify with config tests for a valid table, an empty table equal to none, an unknown name (`max_tokens`), a wrong type, a value out of range, a reload that adds, changes and removes the table, and a refused reload keeping the previous configuration

## 2. Applying the defaults

- [x] 2.1 Convert the table to JSON values once at load and pass it as the backend's `extra_body` in `endpoint_client` (`pool.rs`), leaving the backend's forced `reasoning_effort` unset; verify with an integration test in the style of `tests/drop_fields.rs` against a mock upstream: a default reaches the endpoint when the request lacks the field, the client's value wins when it carries it, an Anthropic-format request gets the defaults too (including `top_k`, which the translation drops), a streaming request gets them, a failover to an endpoint without defaults sends none of the first endpoint's, and an endpoint without `request_defaults` sends the body it did before

## 3. What clients and the operator see

- [x] 3.1 Show the field in the effective configuration (`GET /v1/config` and the admin view serialize the endpoint, so the field appears for the endpoints that have it and is absent for the others); the OpenAPI document describes the configuration view only as "as `GET /v1/config`" and lists no endpoint fields (not even `max_request_tokens`), so it and the generated GUI types need no change; verify with an effective-configuration test that the field is shown with its values and absent on an endpoint without it, and that `/v1/models`, `/v1/key/info` and a completion response do not carry the defaults

## 4. Documentation

- [x] 4.1 Document `request_defaults` in `docs/configuration.md` (what it is for, the allowed fields and ranges, that the client's own value wins, that it is shown in the effective configuration, which fields were measured against Ollama and which were not), and in the routing or provider guide next to `drop_fields`; state what a Claude Code request can and cannot express (its `thinking` object is not translated to an OpenAI-compatible endpoint; `output_config.effort` becomes `reasoning_effort` and wins over the default); verify the strict docs build passes and the documented example loads under `check-config`
- [x] 4.2 Add the 0038 row to the ADR index if it is missing and verify `docs/adr/README.md` lists all 38

## 5. Against the real model

- [x] 5.1 Run a slice of the evaluation task set against a gateway whose `friday` endpoint declares `request_defaults = { reasoning_effort = "none", temperature = 0.7, top_p = 0.8 }`, sending no such fields from the client, and compare with the earlier numbers (79 of 87 with `reasoning_effort` only, 85 of 87 with sampling too); verify every request is answered, none runs to the token limit, and record the measured fields (and whether `top_k`, `min_p` and `presence_penalty` take effect through Ollama's OpenAI-style endpoint) in the configuration guide

## 6. Wrap-up

- [x] 6.1 Check the threat model of `design.md` against what was built (each row true, each proof present, the gap stated in the guide) and record it as `docs/threat-model.md` section B11 with gap 12; verify `docs/architecture.md` needs no change because no component or request step was added; the roadmap row for the shipped item is added by the release change, where the version is known
- [x] 6.2 Run fmt, clippy `-D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items`, the full test suite with and without the `ui` feature, the `ui/` checks and `openspec validate add-endpoint-request-defaults --strict`; verify each exits 0

## Workflow follow-up

- Release: the next minor. A configuration using the field must not reach an older gateway, so bump the image pin before or with the cluster change.
- After release, in the cluster configuration repository: set `request_defaults` on the endpoint of the local thinking model (and on any other local model that needs them), and drop the per-request fields from the evaluation runs.
- Separate changes: `request_overrides` (force a value), defaults on a route or key, mapping the Anthropic `thinking` object to an effort in the translation layer, and recording on a decision which defaults were filled.
