# Tasks

## 1. Actors in the configuration

- [x] 1.1 Add `[actors.<id>]` (`kind` required, `display_name`, `owner`) and `actor =` on a key to the configuration schema and loader, with the checks from the `actors` spec (missing or unknown kind, owner not a human or not defined, owner on a human, key naming an unknown actor, id rules as for keys); verify with loader tests for every scenario in the spec, including that a configuration without `[actors]` loads unchanged
- [x] 1.2 Resolve each key's actor and owner once at load into the configuration snapshot, so a reload applies them to the next request; verify with a test that a key moved to another actor is attributed to the new one after a reload and a request in flight is unaffected
- [x] 1.3 Show the actors in the effective configuration (kind, display name, owner, key ids, labelled `file`) and the actor and owner in `GET /v1/key/info`, with no secret or hash; verify with tests for both endpoints and a check that nothing secret is serialised
- [x] 1.4 Prove an actor grants and removes nothing: a test that gives the same key settings with and without an actor and with a human and an agent, and compares routes, budgets and limits; document `[actors]` in `docs/configuration.md` and verify the documented example loads as written

## 2. Attribution on decisions and in statistics

- [ ] 2.1 Add the actor and owner columns to decisions as an additive ledger migration (next schema version, nullable, no backfill), written with the decision from the resolved key; the owner of a human is that human; verify with migration tests from the previous version and decision tests for an agent with an owner, a human's key, a key without an actor and open mode
- [ ] 2.2 Group admin statistics and the series by `actor` and by `owner` through the existing counting function, with calls that have none in one group named `(none)` and the client `GET /v1/stats` answering 400 as for any unknown `group_by`; verify with ledger tests for the totals, the buckets and the `(none)` group, and an integration test for the client refusal
- [ ] 2.3 Describe the new `group_by` values in `docs/openapi/admin-v1.json`, regenerate `ui/src/api/schema.d.ts`, and update `docs/api.md`; verify the OpenAPI contract test passes and fails when the document and the routes differ

## 3. Revoking a client key

- [ ] 3.1 Add `POST` and `DELETE /admin/v1/keys/{id}/revoke` in the `credentials` scope: an audited, expiring record with an expiry no later than the overrides maximum, stored with the admin revocations, not listed or counted as an override and not removable through the override endpoints; verify with tests for the happy path, an unknown key, the wrong scope, a client key calling it, expiry and removal
- [ ] 3.2 Refuse a request with a revoked key with the same 401 and body as an unknown key, from the same in-memory set an admin revocation uses so no ledger read is added to authentication; mark the key `revoked: true` with its expiry in the effective configuration; verify with a test that compares the revoked and unknown responses byte for byte and a test that the key works again after expiry
- [ ] 3.3 Describe the endpoints in the OpenAPI document and `docs/api.md` (including that deleting the key from the file is the permanent step); verify the contract test and the strict docs build pass

## 4. Request-path guard

- [ ] 4.1 Add an architecture test that no module outside attribution (the decision write, statistics and configuration display) reads an actor or owner, so nothing authorizes, limits or routes by one, and register the new modules in `fitness/register.toml` with their checks; verify the test passes and fails when a read is added to the request path, and that `scripts/select-checks.py` selects the isolation job for the change
- [ ] 4.2 Run the isolation job and the overhead comparison with actors set on the load's keys, and record the result as the next run in `docs/performance.md`; verify the performance gate passes and the figures are within the budgets

## 5. Wrap-up

- [ ] 5.1 Add the actor section to `docs/threat-model.md` (a value from a request never picks an actor; revoke needs `credentials`; the secret never leaves the file) and update `docs/architecture.md` (module table, status table, "Last updated") and `docs/roadmap.md`; verify the Mermaid diagrams still match the code and the strict docs build passes
- [ ] 5.2 Accept ADR 0037 and add it to the `docs/adr/README.md` index; note in `define-ui-config-model` that task 5.4 (database keys) remains for a later change; verify the index lists it
- [ ] 5.3 Run fmt, clippy `-D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items`, the full test suite with and without the `ui` feature, the `ui/` checks and `openspec validate add-actors --strict`; verify each exits 0

## Workflow follow-up

- Release: the next minor (actors in the file, attribution, client-key revoke; nothing changes for existing deployments or client keys).
- Separate changes: the actors and key screens in the GUI; the group-by control gains `actor` and `owner`; a profile and deny lists on an actor; a cap across a person's agents; database-managed keys (the trigger is a second person or a fleet of agents needing keys without the operator); a usage view and sign-in.
