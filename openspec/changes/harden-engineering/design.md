# Design

## Context

Review findings (2026-10-07), measured on `main`:

- Importers of `switchyard_*`: `error`, `pool`, `policy`, `metering`, `pricing`, `routing`,
  `server`. `config`, `auth`, `budget`, `ledger`, `clock` are clean.
- `server::handle` is 119 lines; `config.rs` is 995 lines (about half tests); non-test `unwrap`/
  `expect`: two justified `expect` calls.
- `cargo clippy -W clippy::pedantic` reports about 70 warnings, including roughly 20 numeric casts
  in cost, token and ledger code.
- Mock-upstream and log-capture helpers are copied across 3 to 6 test files.
- CI runs fmt, clippy `-D warnings` and tests only.

## Goals / Non-Goals

**Goals:** make the architecture rules mechanical; keep behavior identical; leave the code easier
to change next time.

**Non-Goals:** new features, performance work, API redesign beyond decoupling `PolicyContext`.

## Decisions

- **Engine boundary.** Switchyard types may appear only in `pool`, `routing`, `metering`,
  `pricing` and `server` (the edge that decodes and encodes protocols) plus `error` (error
  mapping). Everything else (`config`, `auth`, `budget`, `ledger`, `clock`, `policy`) uses
  gateway-owned types. `PolicyContext` therefore drops `Metadata` for `route`, `session_id`, `key`
  and a small `agent` summary. Alternative considered: a full facade crate re-exporting Switchyard;
  rejected as more indirection than the current size warrants, revisit if the allowed set grows.
- **Architecture linter: a plain Rust test (`tests/architecture.rs`), decided by a spike.** The
  spike result is recorded below; the original plan text follows it for context.

  *Spike result (2026-10-07, `mille` 0.0.14 vs a ~250-line scanner test).* `mille` is fast
  (0.02s), has clean output (terminal, JSON, GitHub annotations) and its external-crate rules
  were reliable in every import form (plain, grouped, aliased, `pub use`). Its internal layering
  rules were not: with our flat-file modules a layer needed both `src/x.rs` and `src/x/**` paths
  to match at all, and even then it caught only `use crate::module::Item;` and `pub use`; it
  **missed** `use crate::module::{A, B};`, `use crate::{a, b};`, `use crate::module;` and inline
  paths such as `crate::pool::X` or `switchyard_protocol::Usage` used without a `use`. That is a
  false sense of security for the rule we care most about. The scanner test caught every one of
  those forms (and nested groups), flagged the single real violation (`policy.rs`), checks that
  every module has a layer (a new file fails closed), and doubles as its own regression suite
  because it scans source text passed in as strings. It adds no CI tooling or install time. We
  revisit `mille` if it reaches 1.0 with grouped-import resolution, or if rules become
  structural (crate-level) rather than name-based.

  *Original plan:* try `mille` first, keep a plain-Rust test as the fallback. `mille`
  (github.com/makinzm/mille, `cargo install mille`, `mille.toml`) is a tree-sitter based linter
  with path-glob layers, per-layer allow/deny between layers, and per-layer `external_allow`/
  `external_deny` for crates, which matches our rules directly. It is 0.0.x, so we spike it before
  committing: express the rules below in `mille.toml`, check it passes today (after task 2.1) and
  fails on an injected violation, and time the CI install (pin the version, use `--locked`). If it
  holds up it becomes the enforcement and the decision is recorded as an ADR; if not we write the
  same rules as a plain Rust test scanning `src/`. Rules either way: (1) Switchyard crates only in
  the engine and edge modules, (2) layering: the core modules (`config`, `auth`, `budget`,
  `ledger`, `clock`, `policy`) may not depend on `server`, `pool`, `routing` or `metering`,
  (3) `sqlx` only in `ledger`, (4) no `println!`/`dbg!` outside `main`. Failure messages name the
  rule and point at `docs/invariants.md`. Do not run both: one source of truth.
- **Lints in `Cargo.toml` `[lints]`** so `clippy -D warnings` in CI enforces them: `pedantic` as a
  baseline with a short, justified allow list; `unwrap_used`, `expect_used` and `panic` denied
  (tests allowed through `clippy.toml`); `cast_possible_truncation`, `cast_sign_loss`,
  `cast_precision_loss` denied. Remaining necessary casts move into small, tested helpers
  (`pricing`/`ledger` conversions) with a narrowly scoped `allow` and a reason.
- **`handle()` becomes named stages** (`authorize`, `resolve_route`, `check_budget`, `plan`,
  `execute`, `encode`) each a small function with its own unit tests where logic exists. Stage
  boundaries follow the invariant order in `docs/invariants.md`.
- **Config split** into `config/{mod.rs, schema.rs, validate.rs, routes.rs}`; public API unchanged.
- **Shared test support** in `tests/common/mod.rs` (mock upstream builders, `serve`, log capture).
- **CI additions:** `cargo deny check` (licences allow-list Apache-2.0, MIT, BSD, ISC, Unicode,
  MPL-2.0 where already present; advisories deny; sources restricted to crates.io), `cargo doc
  --no-deps` with `RUSTDOCFLAGS=-D warnings`, an MSRV job at `rust-version`, and `cargo llvm-cov`
  with a line-coverage floor set a few points under the measured value so it ratchets.
- **Property tests** (`proptest`): cost is monotone and linear in token counts up to rounding,
  never overflows for realistic maxima, free prices cost zero; budget state is monotone in spend and
  period rollover only ever resets, never inflates.
- **ADRs use a project workflow schema.** `openspec/schemas/spec-driven-adr` (forked from the
  built-in schema, adapted from the MIT community schema `spec-driven-with-adr`, no companion
  skills) adds an `adr` step after design that records durable decisions as immutable, supersedable
  files in `docs/adr/` (template in `docs/adr/template.md`) and is the default for new changes.
  This change itself started under `spec-driven`, so its ADRs are written by task 6.2:
  use Switchyard in-process; dispatch on Switchyard's client; config-first keys behind a
  `KeyStore` trait; SQLite ledger with async writer; integer micro-USD; one-shot modelrelay
  migration; and the routing-policy seam. Process decisions (OpenSpec workflow, the architecture
  test over `mille`) are recorded in design docs and `docs/invariants.md`, not as ADRs.

## Findings during implementation (lints)

- Numeric conversions now live in `src/num.rs` (checked or saturating, one tested place); the
  `cast_*` lints are denied everywhere else. The helper module needed its own three narrowly scoped
  `allow`s with reasons, the only lossy casts left in the crate.
- Crate-level allows are two: `must_use_candidate` and `missing_errors_doc`, each with a reason in
  `Cargo.toml`. One local `allow(clippy::too_many_lines)` on `server::handle` is temporary and is
  removed by task 4.3.
- Test crates carry a file-level allow for `unwrap`/`expect`/`panic` plus a few readability lints
  (`format_push_string`, `assert_is_empty`, ...), because `allow-*-in-tests` in `clippy.toml` does not
  cover helper functions in integration-test files.
- Existing behavior is unchanged: all 139 tests pass, with `clippy -D warnings` clean.

## Risks / Trade-offs

- Pedantic lints produce churn → allow list kept short and each entry justified; fixes land with
  the lint group so CI stays green.
- Refactoring `handle()` risks behavior drift → the 129 existing tests (including all end-to-end
  suites) must pass unchanged, and no test is edited except for the `PolicyContext` change.
- Coverage floors can be gamed or become annoying → start as a ratchet slightly below the measured
  value and revisit after two changes.
- Architecture tests scan text, so renames can slip past → they check imports by crate name, which
  is stable, and fail closed on unknown modules.

## Migration Plan

Behavior-preserving; land as several small PRs in the task order so each stays reviewable. The only
API change is `PolicyContext` (embedders adjust).

## Open Questions

- Coverage floor value, decided from the first measurement.
