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
- The code sketches in docs/background.md came from an LLM chat and are illustrative; verify
  `switchyard-libsy` APIs against the real crate before using.
- Repo is private for now.
