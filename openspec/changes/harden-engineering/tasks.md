# Tasks

## 1. Invariants and architecture tests

- [ ] 1.1 Write `docs/invariants.md` (pipeline order, engine boundary, layering, secrets, money as integer micro-USD, ledger keyed by key id, no I/O on the response path) and verify each invariant names the test or CI check that enforces it
- [ ] 1.2 Spike `mille`: express the boundary, layering, `sqlx` confinement and `println!`/`dbg!` rules in `mille.toml`, run it locally, and verify it flags today's `policy.rs` and an injected violation; if it is unusable or flaky, write the same rules as a plain Rust architecture test instead; record the outcome and reasoning in design.md
- [ ] 1.3 Wire the chosen linter into CI (pinned and locked if `mille`) and verify a deliberate violation on a scratch branch fails the job; expected to fail on `policy.rs` until task 2.1 lands, so merge them together

## 2. Decoupling

- [ ] 2.1 Replace `Metadata` in `PolicyContext` with gateway-owned fields and update `policy`, `server` and the policy tests; verify the import-boundary test passes and all tests are green

## 3. Lints

- [ ] 3.1 Add the `[lints]` tables and `clippy.toml`, fix `unwrap`/`expect`/`panic` findings, and verify `cargo clippy --all-targets -- -D warnings` passes
- [ ] 3.2 Move the money and token casts into tested conversion helpers (saturating or checked) and deny the cast lints; unit tests cover boundary values (`u64::MAX`, negative `i64` from SQLite)
- [ ] 3.3 Work through the remaining pedantic findings (fix or justified allow) and verify the allow list in `Cargo.toml` is under ten entries, each with a reason comment

## 4. Structure

- [ ] 4.1 Share test scaffolding in `tests/common` and update the test files to use it; verify the same test count passes and duplicate helper definitions are gone (grep)
- [ ] 4.2 Split `config.rs` into schema and validation modules with unchanged public API; verify all config tests pass untouched
- [ ] 4.3 Split `handle()` into named stages with the invariant order; verify every end-to-end suite passes unchanged and no stage exceeds the `too_many_lines` limit

## 5. CI

- [ ] 5.1 Add `deny.toml` and a `cargo deny` job; verify it passes and fails on a deliberately banned licence in a scratch branch
- [ ] 5.2 Add a docs job (`cargo doc --no-deps` with warnings denied) and an MSRV job at the declared `rust-version`; verify both run green in CI
- [ ] 5.3 Add a coverage job with `cargo llvm-cov`, record the baseline and set the floor a few points below it; verify the job reports and enforces

## 6. Property tests and ADRs

- [ ] 6.1 Add property tests for pricing and budget state/rollover; verify they run in CI and shrink a deliberately injected bug locally
- [ ] 6.2 Write the ADRs (the six existing decisions plus the linter choice) in `docs/adr/` from `docs/adr/template.md`, following the adr step's rules; verify each links to its OpenSpec change and to the invariant it supports and that numbers are sequential
- [ ] 6.3 Update `AGENTS.md` and `docs/architecture.md` to point to the invariants, the architecture tests and the ADRs; verify the links resolve
