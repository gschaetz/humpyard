# Tasks

Each group stands alone and ships in its own change; this change records the decisions only.

- [x] 1.1 ADR 0025, `docs/architecture.md` and `docs/roadmap.md` updated (done in this change's PR)
- [x] 2.1 Effective-configuration endpoint with provenance, and the fingerprint as an endpoint and a metric (shipped as `add-effective-config`, archived 2026-10-20)
- [x] 3.1 Override store (ledger table, expiry, audit rows), planning input, `explain`/decision marks, `overrides_active` metric and alert example (shipped as `add-runtime-overrides`, archived 2026-10-23, release 0.13.0)
- [ ] 4.1 Read-only status page on the same API (now the first view of the admin GUI: a separate client of `/admin/v1`, which shipped as `add-admin-api`; to be proposed as its own change once the GUI's hosting and stack are chosen)
- [ ] 5.1 Patch generation with a format-preserving TOML editor, validated by the real loader
- [ ] 5.2 Optional direct write (`[admin] allow_config_writes`), fingerprint-conditional and atomic
- [ ] 5.3 Key profiles in the configuration (`[profiles.*]`: routes, classes, budget ceilings, lifetime, `max_active_keys`) and `profile =` on config keys
- [ ] 5.4 Database-managed keys: ledger table, admin-only create (secret shown once), narrowing to `min(key, profile)`, mandatory expiry and (for paid-capable profiles) budget, `max_active_keys`, immediate revoke with an audit row
- [ ] 5.5 Gate before sharing keys with anyone but the operator: a per-profile rate limit or concurrency cap
- [ ] 6.1 Architecture test: only the file writer and the override store can change effective configuration
