# Tasks

## 1. The scope and the dependencies

- [x] 1.1 Add `toml_edit` and `similar` to `Cargo.toml` and add `AdminScope::Config` (in `ALL`, parsed, displayed, accepted by `humpyard keygen --admin --scopes`, shown by `check-config`); verify `cargo deny check` passes, config tests for a credential listing `config` and for an unknown scope still being refused, and a keygen test that the printed block loads
- [x] 1.2 Document the scope in `docs/configuration.md` (the `[admin]` bullet) and `docs/api.md` (the admin scopes table); verify the strict docs build

## 2. Editing and checking

- [ ] 2.1 Move the load-then-compare sequence of `try_reload` into one function, `validate_text`, used by `try_reload` and by the patch, including the rule that a patch is refused when the running configuration merged the generated file and the patched one would not; verify the existing reload tests pass unchanged and a new test shows each restart-only setting refused through both paths
- [ ] 2.2 Add `src/config/patch.rs`: the typed edits (`set`, `remove`, `add_table`, `remove_table`), the section allowlist (under `keys` only `set`/`remove`, never `sha256`, never adding or removing a key), path and name validation, the limits (64 edits, depth 8, value nesting 4), and their application with `toml_edit`; verify unit tests for each op, comments and ordering kept (every untouched line byte-identical), every forbidden path, a missing path, an existing table on `add_table`, names and values containing quotes, newlines and brackets, and each limit at the boundary and one over
- [ ] 2.3 Add the masking step (key hashes, credential hashes and provider header values replaced by `<redacted>` by walking the parsed document) and the unified diff with three lines of context; verify unit tests that removing a provider with headers shows redacted values, that every `sha256` in the file is masked in a diff that touches its table, that an edit that changes nothing gives an empty diff, and that the diff is stable across runs

## 3. The endpoint

- [ ] 3.1 Store the configuration file's path in `AppState`, add `POST /admin/v1/config/patch` (scope `config`, 64 KiB body cap, fingerprint of the file's text with the generated file as a reload reckons it, then the edits, then `validate_text`) with the answers 200, 400 `invalid_edit`, 403 `admin_scope_required`, 409 `stale_fingerprint`, 422 `config_rejected` and 503 `config_file_unavailable`, one log line with the admin id, edit count and outcome, and the entry in the OpenAPI document; verify the route-versus-document test and a new `tests/config_patch.rs` covering every scenario in the `config-patch` and `admin-api` specs (including `config` alone failing on `GET /admin/v1/config`, a stale base after a file edit and after an unreloaded file edit, identical repeated responses, and the file's bytes, fingerprint and reload status unchanged after each outcome)
- [ ] 3.2 Mutation-check the guarantees and revert each: let a forbidden section through, skip the fingerprint check, validate without the restart-only checks, leave a header value unmasked, write the file after a valid patch; verify each is caught by a test

## 4. Docs and wrap-up

- [ ] 4.1 Document the endpoint in `docs/api.md` (request, edit forms, answers, limits, the diff being for review), add the invariant "a patch writes nothing" to `docs/invariants.md`, add `docs/roadmap.md` (unreleased) and update `docs/architecture.md` (the admin surface, status table, "Last updated"); verify the strict docs build and that its Mermaid diagrams still match the code
- [ ] 4.2 Check the design's threat model against what was built (each of the nine rows still true, each proof exists) and update `docs/threat-model.md` (B3 and B4, and the accepted gap in row 8); verify every row cites a test or invariant that exists
- [ ] 4.3 Run fmt, clippy `-D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --document-private-items` and the full test suite; set ADR 0034 to accepted, tick 5.1 in `define-ui-config-model`, and archive with `openspec archive add-config-patch --yes`; verify all gates pass and the spec deltas merge

## Workflow follow-up

- Release notes: new and off unless a credential holds `config`; nothing is written by it.
- Later, each its own change: the direct write (5.2, reusing the edits and the fingerprint), editing `select` rules, and the GUI (4.1).
