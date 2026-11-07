# Tasks

## 1. Configuration

- [x] 1.1 Add the `[aliases.<id>]` table (`model`, `display_name`, `description`) to `RawConfig` and `Config`, with `deny_unknown_fields`; verify an existing config with no aliases still loads (existing config tests pass)
- [x] 1.2 Validate aliases at load: id shape, no collision with routes, targets or other aliases, `model` names a route or target (never an alias), label limits; verify with one config test per rejection in the gateway-config spec
- [x] 1.3 Add the top-level `model_aliases` switch (default `true`); verify config tests cover absent, `true` and `false`, and that an invalid alias is still rejected when `false`
- [x] 1.4 Reject an alias named in a key's `allowed_routes`; verify with a config test
- [x] 1.5 Document the table in `docs/configuration.md` and verify the example config in the docs loads (`humpyard check` or the docs test used for other examples)

## 2. Gateway

- [x] 2.1 Resolve an alias to its real name where the requested model is read, before `select_route`, for inference and token-counting endpoints; verify an alias request is served by the real route and the decision record and ledger row hold the real name
- [x] 2.2 List aliases in `list_models` filtered by `may_use(target)`, with `display_name` and `description` when set; verify real-name entries are byte-for-byte unchanged and a restricted key does not see a forbidden pool's alias
- [x] 2.3 Verify a key without the target is refused for its alias with the real name in the error, and selector rules keyed on `model` match via the alias (tests in `tests/`)
- [x] 2.4 With `model_aliases = false`, verify an alias request gets model-not-found and `/v1/models` lists no alias
- [x] 2.5 Verify a reload that changes aliases applies to new requests only (extend the reload test)
- [x] 2.6 Update `docs/api.md` for the `/v1/models` fields and alias requests, and verify the documented request runs as written

## 3. Docs and integration

- [x] 3.1 Add a Claude Code discovery recipe to `docs/clients.md` (`CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY=1`, `claude-` aliases, labels) and verify it by hand against a running gateway: the aliases appear in `/model` and selecting one routes to the pool; record the Claude Code version
- [x] 3.2 Update `docs/architecture.md` (request flow: alias resolution at decode; its "Last updated" line) and verify its Mermaid diagrams still match the code
- [x] 3.3 Check the design's threat model against what was built (each row true, each proof exists) and update `docs/threat-model.md`
- [x] 3.4 Run the CI gates locally (`cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `tests/architecture.rs`) and `openspec validate gateway-model-aliases`

## Workflow follow-up

- Archive the change in the PR that lands its last task: `openspec archive gateway-model-aliases --yes`.
- Commits need a DCO sign-off (`git commit -s`); `main` is PR-only.
