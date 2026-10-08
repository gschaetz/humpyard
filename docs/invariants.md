# Architecture invariants

Rules that must stay true as the code changes. Each names what enforces it, so a violation fails a
build instead of relying on review. When a rule changes, change it here, in the enforcing test and
in an ADR (`docs/adr/`) in the same change.

## Structure

1. **Engine boundary.** Switchyard crates (`switchyard_libsy`, `_protocol`, `_translation`,
   `_llm_client`, `_runner`) are used only by the engine and edge modules (`error`, `pricing`,
   `pool`, `routing`, `metering`, `server`). Core modules (`clock`, `config`, `policy`, `auth`,
   `ledger`, `budget`) use gateway-owned types. Reason: Switchyard is pre-1.0; its churn must stay
   in a few files. *Enforced by* `tests/architecture.rs`.
2. **Layering.** Core modules may reference only the modules listed for them in the rules table
   (for example `budget` may use `clock`, `config`, `ledger`, `policy`); engine modules may not
   reference the HTTP edge (`server`). *Enforced by* `tests/architecture.rs`; a new module fails
   the test until it is given a layer.
3. **SQL stays in the ledger.** Only `src/ledger.rs` uses `sqlx`. *Enforced by* `tests/architecture.rs`.
4. **Only `main` prints.** Everything else logs through `tracing`. *Enforced by* `tests/architecture.rs`.

## Supply chain and builds

11. **Dependencies stay permissive, advisory-free and from crates.io.** Licences outside the
    allow-list in `deny.toml`, known vulnerabilities, yanked crates and non-crates.io sources fail
    the build. *Enforced by* the `deny` CI job (`cargo deny check`).
12. **The declared minimum Rust version builds** (`rust-version` in `Cargo.toml`), the docs build
    without warnings, and line coverage does not fall below the floor (currently 92%; baseline
    96.15% on 2026-10-07). The floor only ratchets up. *Enforced by* the `msrv`, `docs` and
    `coverage` CI jobs.

13. **Every commit is signed off (DCO).** Each non-merge commit in a pull request carries a
    `Signed-off-by` line, as `CONTRIBUTING.md` requires. *Enforced by* the `dco` CI job.

## Request path

5. **Order of checks.** A request is authenticated, checked against the key's route allowlist, and
   checked against its budget *before* any upstream call is made. *Enforced by* `tests/auth.rs`
   (401/403 make no upstream call) and `tests/budget.rs` (402 makes no upstream call).
6. **Routing never sees ineligible targets.** The policy narrows targets before any algorithm
   runs. *Enforced by* `tests/policy.rs`.
7. **Ledger writes never block a response.** Entries go through a bounded queue with `try_send`; a
   full queue or failing database is counted, not surfaced. *Enforced by* the ledger tests
   (`a_full_queue_drops_and_counts_instead_of_waiting`, `a_broken_database_never_blocks_or_fails_recording`).

## Data and secrets

8. **No plaintext secrets.** Provider keys come from environment variables; client keys exist only
   as SHA-256 hashes in config; neither keys nor hashes appear in logs. *Enforced by* the config
   tests, `tests/auth.rs` (log checks) and `Debug` redaction tests.
9. **Money is integer micro-USD, and numeric conversions are checked.** Cost is computed and
   stored as whole micro-USD; floating point appears only at the config and display edges, and
   every integer/float conversion on money and token paths goes through `src/num.rs`. *Enforced
   by* the denied `clippy::cast_*` lints in `Cargo.toml` (run by CI), the `num` and `pricing`
   tests, and, for production code, denied `unwrap_used`, `expect_used` and `panic`. Property
   tests are planned (`harden-engineering` group 6).
10. **The ledger refers to key ids, never hashes.** Identity changes (for example database-managed
    keys) must not require a ledger migration. *Enforced by* review and the ledger schema; see
    [ADR-0003](adr/0003-config-first-virtual-keys-behind-keystore.md).
