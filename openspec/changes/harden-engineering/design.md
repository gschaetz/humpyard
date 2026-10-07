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
- **Architecture tests are plain Rust tests** scanning `src/` (no extra tooling): (1) Switchyard
  imports only in the allowed modules, (2) layering: the "core" modules (`config`, `auth`,
  `budget`, `ledger`, `clock`, `policy`) may not `use crate::{server, pool, routing, metering}`,
  (3) `sqlx` only in `ledger`, (4) no `println!`/`dbg!` outside `main`. Each failure message
  names the rule and points at `docs/invariants.md`.
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
- **ADRs** (`docs/adr/NNNN-title.md`, one page each): use Switchyard in-process; dispatch on
  Switchyard's client; config-first keys behind a `KeyStore` trait; SQLite ledger with async writer;
  integer micro-USD; one-shot modelrelay migration.

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
