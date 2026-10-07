# switchyard-conductor — agent context

Rust single-binary LLM gateway combining budget/cost tracking, NVIDIA NeMo Switchyard routing
(in-process via `switchyard-libsy`), and a Rust port of modelrelay's provider pool.
See [README.md](README.md) and [docs/background.md](docs/background.md) for design intent.

## Workflow
- Spec-driven with **OpenSpec** (`openspec/`): propose changes with `/opsx:propose`, specs in
  `openspec/specs/`, active changes in `openspec/changes/`. Write specs before code.
- Project constraints for OpenSpec artifacts live in `openspec/config.yaml`.

## Gotchas
- Sibling repo `../modelrelay` is the Node.js fork (LTS); this repo is the long-term successor.
  Must import its `~/.modelrelay.json` format.
- Pipeline order (budget -> Switchyard -> dispatch) is deliberate; don't reorder.
- Switchyard crates (`switchyard-libsy`, `-protocol`, `-translation`, ...) are on crates.io, Apache-2.0.
  Findings on their real API are in docs/background.md; the original chat's code sketches were
  illustrative and wrong in places (e.g. no budget/telemetry input, targets are model ids not tags).
