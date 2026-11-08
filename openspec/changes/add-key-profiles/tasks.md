# Tasks

## 1. Profiles in the configuration

- [ ] 1.1 Add `[profiles.<id>]` (`allowed_routes`, `over_budget`, the four budgets, `max_request_usd`, `max_requests_per_minute`, `max_concurrent`; any other setting refused) and `profile =` on a key; make `over_budget` on a key optional so an omitted value inherits; verify config tests for a valid profile, an unknown setting such as `read_config`, an unknown profile on a key, and an invalid value in a profile naming the profile and the setting
- [ ] 1.2 Resolve a key against its profile at load (inherit what is unset; refuse a wider number, a route outside the profile's list, and `free_only` under `block`, naming key, profile and setting), keep `KeyConfig` as the resolved shape plus the profile name; verify config tests for each rule in the key-profiles spec (lower allowed, wider refused, subset allowed, outside refused, inheritance, no bound when the profile leaves it unset, no profile unchanged)
- [ ] 1.3 Show `profile` on each key and a `profiles` object in `GET /v1/config`; verify `tests/effective_config.rs` for a key with a profile (resolved values, the profile name, no hash) and for the listed profiles
- [ ] 1.4 Verify end to end through the gateway that an inherited `max_requests_per_minute` is enforced (the N+1th request is 429), that a reload raising the profile's limit admits the next request, and that a reload which would widen a key past its profile is refused and leaves the previous configuration serving

## 2. Docs and wrap-up

- [ ] 2.1 Document key profiles in `docs/budgets.md` (the section on keys) and `docs/configuration.md`, including that a key profile is unrelated to the `x-humpyard-profile` header; verify the strict docs build
- [ ] 2.2 Update `docs/architecture.md` (status table, "Last updated", the config row in the module table if it changed) and verify its Mermaid diagrams still match the code; check the design's threat model against what was built (each row true, each proof present), record it in the pull request, and update `docs/threat-model.md` for the new gap row
- [ ] 2.3 Run fmt, clippy `-D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --document-private-items` and the full test suite, then archive with `openspec archive add-key-profiles --yes`; verify all gates pass and the spec deltas merge

## Workflow follow-up

- Release notes: new, off by default; existing configurations are unaffected.
- Next, each its own change: database-managed keys (5.4, which brings `allowed_classes`, the maximum lifetime and `max_active_keys`), the per-profile sharing gate (5.5).
