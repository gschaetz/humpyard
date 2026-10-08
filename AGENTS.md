# humpyard — agent context

Rust single-binary LLM gateway combining budget/cost tracking, NVIDIA NeMo Switchyard routing
(in-process via `switchyard-libsy`), and an original provider pool (modelrelay-style concepts, no shared code).
See [README.md](README.md) and [docs/background.md](docs/background.md) for design intent.

Naming: formerly `switchyard-conductor`, renamed to humpyard on 2026-10-07 (a hump yard is the railroad
yard that classifies cars onto the right track). Archived OpenSpec changes and ADR 0008 (immutable)
keep the old `x-conductor-*` header names as history; living specs and code use `x-humpyard-*`.

## Workflow
- Spec-driven with **OpenSpec** (`openspec/`): propose changes with `/opsx:propose`, specs in
  `openspec/specs/`, active changes in `openspec/changes/`. Write specs before code.
- Project constraints for OpenSpec artifacts live in `openspec/config.yaml`.
- Workflow schema: `spec-driven-adr` (project-local, `openspec/schemas/`): proposal → specs → design →
  adr → tasks. The `adr` step records long-term architectural commitments as **immutable**,
  supersedable ADRs in `docs/adr/` (never edit an accepted ADR; add a new one that supersedes it).
  Tactical choices stay in `design.md`. Changes started before the schema keep their own schema.

- Keep [docs/architecture.md](docs/architecture.md) current: any PR that changes structure, request
  flow or component status updates its diagrams and status table, and its "Last updated" line.

## Guardrails (read before changing structure)
- [docs/invariants.md](docs/invariants.md) lists the rules that must stay true and what enforces
  each. `tests/architecture.rs` fails the build on layering/import violations (Switchyard types
  only in engine and edge modules, `sqlx` only in the ledger, no prints outside `main`, every new
  module needs a layer); add new modules to `rules()` there.
- Durable decisions are in [docs/adr/](docs/adr/) (immutable; supersede, never edit).
- Lints (see `Cargo.toml`): no `unwrap`/`expect`/`panic` in production code, and no numeric `as`
  casts on money or token paths: use `src/num.rs`. Tests live in `tests/`; shared helpers are in
  `tests/common/`.
- CI gates (all required on `main`): `check` (fmt, clippy `-D warnings`, tests), `deny` (licences,
  advisories), `docs` (warning-free), `msrv` (Rust 1.96.1), `coverage` (floor 92% lines, only ever
  raised). `main` is PR-only.
- Commits need a DCO `Signed-off-by` line (`git commit -s`, see CONTRIBUTING.md); the `dco` job
  fails a PR with an unsigned commit. Fix with `git rebase --signoff origin/main`.

## Releases
- Releases are tag-only (`vX.Y.Z` matching `Cargo.toml`, commit on `main`; ADR 0010,
  [docs/deployment.md](docs/deployment.md)). Pushing a tag publishes binaries and an image publicly,
  so never create or push a release tag unless the user explicitly asks for that release.

## Gotchas
- Sibling repo `../modelrelay` is a separate Node.js project (a fork, not a code source for this
  repo; keep it that way for licensing). Migration is a one-shot `migrate-modelrelay` command, not a runtime reader of
  `~/.modelrelay.json` (see docs/background.md, "modelrelay migration").
- The request path order is fixed (decode → authorize → budget check → policy → Switchyard run →
  encode; [docs/invariants.md](docs/invariants.md) item 5). Nothing reaches an upstream before the
  first four succeed; don't reorder the stages in `server::handle`.
- Switchyard crates (`switchyard-libsy`, `-protocol`, `-translation`, ...) are on crates.io, Apache-2.0.
  Findings on their real API are in docs/background.md; the original chat's code sketches were
  illustrative and wrong in places (e.g. no budget/telemetry input, targets are model ids not tags).
