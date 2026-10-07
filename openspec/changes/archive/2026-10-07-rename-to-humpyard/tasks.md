# Tasks

## 1. Rename

- [x] 1.1 Rename crate, library and binary to `humpyard` and update imports in `src/`, `tests/` and `Cargo.toml` (adding `repository` and `homepage`); verify `cargo build`, `cargo clippy -D warnings` and `cargo test` pass and `humpyard --version` runs
- [x] 1.2 Rename the public identifiers (`x-humpyard-*` headers, `sk-humpyard-` key prefix, `owned_by`, example ledger file) and update the tests that assert them; verify the header, keygen and model-list tests pass against the new values
- [x] 1.3 Update docs, README, examples, `NOTICE`, `AGENTS.md` and the architecture doc, and add a short "why humpyard" note; verify with `git grep -i "switchyard-conductor\|x-conductor\|sk-conductor"` that only archived changes remain
- [x] 1.4 Verify the documented quickstart commands and `check-config` on the examples still run under the new binary name
