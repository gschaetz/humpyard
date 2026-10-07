# Proposal

## Why

The gateway grew fast through four changes and the architecture rules lived in prose. A review
found drift: the Switchyard (pre-1.0) crates are imported by seven modules, not the one boundary we
intended; `handle()` is 119 lines orchestrating auth, routing, budgets, metering and encoding;
test scaffolding is copied across files; CI lacks dependency, docs, MSRV and coverage checks; and
the money and token paths use unchecked numeric casts. Fixing this before more features is cheaper
than after, and the rules should be enforced by tests and CI, not by discipline.

## What Changes

No runtime behavior changes. This change:

- Writes the **architecture invariants** down (`docs/invariants.md`) and enforces them with
  architecture tests (allowed importers of Switchyard crates, module layering, SQL confined to the
  ledger).
- Decouples `policy` and any non-engine module from Switchyard types.
- Adds **lints** (clippy pedantic baseline, `unwrap`/`expect`/`panic` denied outside tests, numeric
  cast lints on the money and token paths) and fixes the findings.
- **Refactors** for testability: split `config.rs` into schema and validation, split `handle()`
  into named stages, and share test scaffolding in `tests/common`.
- Strengthens **CI**: dependency licences and advisories (`cargo deny`), a docs build with warnings
  denied, an MSRV job, and a coverage report with a floor.
- Adds **property tests** for pricing and budget arithmetic and records the key decisions as short
  **ADRs**.

Out of scope: new features, spec-kit migration, replacing Switchyard.

## Capabilities

### New Capabilities

### Modified Capabilities

(none: a behavior-preserving refactor and tooling change, so `skip_specs` is set)

## Impact

- `src/` is reorganized (config split, handle stages, policy context type) with every existing test
  still passing unchanged in behavior; new architecture and property tests; new CI jobs and config
  files (`deny.toml`, `clippy.toml`, lint tables in `Cargo.toml`); new docs.
- `PolicyContext` loses its raw `Metadata` field in favor of gateway-owned fields (embedders
  of the pre-alpha API adjust).
