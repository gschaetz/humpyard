# switchyard-conductor — agent context

Rust single-binary LLM gateway combining budget/cost tracking, NVIDIA NeMo Switchyard routing
(in-process via `switchyard-libsy`), and an original provider pool (modelrelay-style concepts, no shared code).
See [README.md](README.md) and [docs/background.md](docs/background.md) for design intent.

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

## Gotchas
- Sibling repo `../modelrelay` is a separate Node.js project (a fork, not a code source for this
  repo; keep it that way for licensing). Migration is a one-shot `migrate-modelrelay` command, not a runtime reader of
  `~/.modelrelay.json` (see docs/background.md, "modelrelay migration").
- Pipeline order (budget -> Switchyard -> dispatch) is deliberate; don't reorder.
- Switchyard crates (`switchyard-libsy`, `-protocol`, `-translation`, ...) are on crates.io, Apache-2.0.
  Findings on their real API are in docs/background.md; the original chat's code sketches were
  illustrative and wrong in places (e.g. no budget/telemetry input, targets are model ids not tags).
