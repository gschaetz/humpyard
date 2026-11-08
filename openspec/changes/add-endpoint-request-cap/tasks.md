# Tasks

## 1. The setting

- [x] 1.1 Add `max_request_tokens` to an endpoint (optional; refused when zero, negative or not a whole number, naming the setting, or above a declared `context_window`, naming target, endpoint and setting) and make `declares_context_windows` true for an endpoint that declares it; verify config tests for a valid cap beside a large window, no cap, zero, negative, a fraction, and a cap above the window, and that a pool with `min_context_window = 32000` still includes a target with `context_window = 131072` and `max_request_tokens = 8000`
- [x] 1.2 Check that the effective configuration (`GET /v1/config`) and `humpyard ctl config` show the cap on an endpoint and omit it when unset, and that `prices import` cannot propose or overwrite it (the importer's `Capability` has no cap field); verify `tests/effective_config.rs`

## 2. Planning

- [x] 2.1 Remove a target for the cap in `incapable_reason` with the reason `request_cap` (low estimate above the cap of every endpoint of the target, prompt only, same certainty rule as the window), add `request_cap` to the bounded reasons, give the refusal sentence and `explain` the reason; verify unit tests for a prompt over, at and under the cap, a target with one capped and one uncapped endpoint staying, and a cap-only configuration measuring the prompt
- [x] 2.2 Verify through the gateway that an oversized prompt never reaches a capped target while a second target serves it, a small prompt is served by the capped target on a large-window pool route, an all-capped route answers 400 `no_capable_target` naming `max_request_tokens` with no upstream call, and the decision, `explain` and `humpyard_capability_skips_total{reason="request_cap"}` show the removal; verify the new integration tests (in `tests/capability_eligibility.rs`)

## 3. Docs and wrap-up

- [x] 3.1 Document the cap in `docs/routing.md` (Capabilities, and the Groq example with a window and a cap), `docs/configuration.md` and `docs/metrics.md`, and update `docs/architecture.md` (status table, "Last updated"); verify the strict docs build and that its Mermaid diagrams still match the code
- [ ] 3.2 Run fmt, clippy `-D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --document-private-items` and the full test suite, then archive with `openspec archive add-endpoint-request-cap --yes`; verify all gates pass and the spec deltas merge

## Workflow follow-up

- Release notes: new, off by default; the Groq stopgap (`context_window = 8000`) can be replaced by the real window plus `max_request_tokens = 8000` after upgrading.
- Later, each its own change: whether a provider counts the requested output toward its cap, skipping a single capped endpoint inside a target, and a per-minute token allowance.
