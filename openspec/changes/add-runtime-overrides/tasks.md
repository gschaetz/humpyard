# Tasks

- [x] 1.1 `[overrides]` settings and the `manage_overrides` key setting: schema, validation, docs
- [x] 1.2 Ledger tables `overrides` and `override_audit`, the in-memory active set, expiry noticed lazily with an audit row
- [x] 1.3 Planning: overrides as a subtract-only input (after strictness and capabilities, before health); `override_no_target`
- [x] 1.4 `POST`/`GET`/`DELETE /v1/overrides` with validation (existing subject, a pin never adds reach or crosses a class, bounded expiry, ledger required) and the 403/409 rules
- [ ] 1.5 Decisions, `explain` and `GET /v1/config` show the overrides (source `override`)
- [ ] 1.6 Metrics `humpyard_overrides_active` and `humpyard_override_oldest_age_seconds`, and the `HumpyardOverrideLong` alert example
- [ ] 1.7 `ctl overrides list|add|remove`
- [ ] 1.8 Tests for every scenario, including an architecture test that only the override store changes the planning input, and that no override can add an entity
- [ ] 1.9 Docs (`api.md`, `ctl.md`, `routing.md`, `metrics.md`, `budgets.md`), `docs/architecture.md`; archive this change
