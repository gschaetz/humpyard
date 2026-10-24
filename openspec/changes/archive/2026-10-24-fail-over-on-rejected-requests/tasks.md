# Tasks

## 1. Class and configuration

- [x] 1.1 `FallbackClass::Rejected` (`rejected` in config and reports) and its default on `fastest` and pool routes only
- [x] 1.2 `fallback_class` classifies a 400/422 that is not model-gone as `rejected`; a test pins the exact statuses across 400 to 451

## 2. Request path

- [x] 2.1 `Metered` re-labels a `rejected` answer so Switchyard moves on, and restores the original when no target answers
- [x] 2.2 Stop at the third distinct provider that rejected the request, counted from the request's attempts
- [x] 2.3 The pool moves to the next endpoint of the target on `rejected`; no health count, no speed sample

## 3. Tests and docs

- [x] 3.1 Integration tests: strict first member then healthy; three providers reject; passthrough unchanged; opt-in; tail guarded; model-gone 400 still `unavailable`; endpoint health untouched
- [x] 3.2 Document the class in docs/routing.md and docs/configuration.md; update the Codex notes in docs/clients.md
- [x] 3.3 Update docs/architecture.md and docs/invariants.md if the failover description or its enforcement changes
- [x] 3.4 Mark ADR 0026 accepted when the change is archived
