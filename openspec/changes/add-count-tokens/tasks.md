# Tasks

## 1. Estimator

- [ ] 1.1 Implement `estimate_tokens(&Value) -> u64` as a pure core module with a documented bytes-per-token constant and per-message overhead; unit tests for empty, text, multi-byte, tool-definition and nested-content inputs; property tests: never panics, monotone in appended content, additive within the overhead; add the module to `tests/architecture.rs`
- [ ] 1.2 Calibrate the constant against real providers using their reported `prompt_tokens` for prompts of several kinds (English, code, JSON tool schemas, non-ASCII); record the ratios in design.md and choose a constant that errs high for all of them

## 2. Endpoint

- [ ] 2.1 Add the `count_tokens` handler reusing authenticate, decode and authorize and skipping budget and policy; response `{"input_tokens": n}` plus the estimate header; tests: a body without `max_tokens` counts, more content counts more, 401, 403 for a disallowed model, 404 for an unknown model, 400 for bad JSON, all in Anthropic's error shape
- [ ] 2.2 Test that no upstream call and no ledger entry happen, and that a key at its budget limit still gets a count

## 3. Docs

- [ ] 3.1 Document the endpoint and its estimate semantics in the README and `docs/clients.md` (remove the "not implemented" note), update `docs/architecture.md` and the invariants doc if a rule changes; verify a real Claude Code `/context`-style call or a curl against the running gateway returns a count
