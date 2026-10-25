# Tasks

## 1. Configuration

- [x] 1.1 `[admin]` and `[admin.credentials.<id>]` in the schema and loader: hash, scopes, optional `expires`, optional `listen`; verify with config tests for a valid section, no section, `listen` without credentials, and an empty scope list
- [x] 1.2 Load-time validation: unknown scope, duplicate hash, a hash equal to a client key's or the metrics credential's, an unparsable `expires`; verify each is refused with the offending id named
- [x] 1.3 Credentials in the configuration snapshot and fingerprint, shown by id, scopes and expiry only in `GET /v1/config` and `check-config`; verify a test that no hash or secret appears and that the fingerprint changes when a credential does
- [x] 1.4 Document `[admin]` in `docs/configuration.md` and verify the example in it loads with `check-config`

## 2. Authentication and the namespace

- [ ] 2.1 The admin authenticator over the snapshot's credentials (expiry against the clock, constant shape of failure), separate from the client `KeyStore`; verify unit tests for valid, unknown, expired and wrong-kind credentials
- [ ] 2.2 `/admin/v1` router merged into the main one, or served by its own listener when `[admin] listen` is set, and absent (404) without credentials; verify tests for the 404, the own-listener case and that the main listener 404s when `listen` is set
- [ ] 2.3 Scope checks with 401 `admin_credential_invalid` (with `WWW-Authenticate`) and 403 `admin_scope_required` naming the scope, plus `GET /admin/v1/whoami`; verify tests that a client key and an admin credential each fail on the other's endpoints
- [ ] 2.4 `humpyard_admin_requests_total{result}` exported only when the admin API is enabled; verify a metrics test and update `docs/metrics.md`
- [ ] 2.5 Credentials follow a reload (add, remove, narrow scopes, and a rejected reload keeps the old set); verify a reload test

## 3. All-keys reads

- [ ] 3.1 Make the cores of config view, health, quota, decisions and stats callable by both surfaces, with the key filter passed in; verify the existing client-endpoint tests still pass unchanged
- [ ] 3.2 `GET /admin/v1/config`, `/health`, `/quota`, `/decisions`, `/stats` with the `read` scope and the optional `key` filter; verify a test with two keys that the admin sees both and the filter narrows
- [ ] 3.3 `GET /admin/v1/credentials`: id, scopes, expiry and revocation state only; verify a test that no hash appears
- [ ] 3.4 Document the read endpoints in `docs/api.md` and verify the documented requests run against a test gateway

## 4. Overrides and revocation

- [ ] 4.1 `/admin/v1/overrides` (list with `read`; create and remove with `overrides`) on the same store and rules, recording the creator as `admin:<id>`; verify tests that the rules match `/v1/overrides` and the audit row names `admin:gary`
- [ ] 4.2 The `revoke_admin_credential` override kind and `POST /admin/v1/credentials/{id}/revoke` (`credentials` scope, expiry within the maximum, unknown id refused); verify the next request by the revoked credential is refused without a reload and works again after expiry or removal
- [ ] 4.3 Keep revocations out of planning, `explain`, decisions, `GET /v1/overrides` and `humpyard_overrides_active`, and add `humpyard_admin_revocations_active`; verify tests for each exclusion and the gauge
- [ ] 4.4 Extend the architecture test so only the pool and the server read overrides still holds with the admin module, and add admin routes to it; verify `cargo test --test architecture` passes
- [ ] 4.5 Document revocation and the audit identity in `docs/api.md` and `docs/routing.md#runtime-overrides`, and add an alert example for an old revocation to the alerts file; verify the alert file lints as the others do

## 5. The contract

- [ ] 5.1 One route table (method, path, scope) from which the router is built, and `docs/openapi/admin-v1.json` served at `GET /admin/v1/openapi.json`; verify the endpoint returns the document to any valid credential
- [ ] 5.2 A test that fails when the table and the document differ in either direction, in scope or in error codes; verify by temporarily removing a route and seeing it fail
- [ ] 5.3 Add `docs/openapi/` to the docs build or link it from `docs/api.md` so the site build stays strict-clean; verify the `docs` job's strict build passes locally

## 6. Operator experience and wrap-up

- [ ] 6.1 `keygen --admin <id> --scopes ...` printing a secret once and the `[admin.credentials.<id>]` block with its hash; verify a test that the printed block loads and authenticates
- [ ] 6.2 Update `docs/architecture.md` (component table, the Direction section and its diagrams) and `docs/roadmap.md`, and verify the Mermaid diagrams still match the code
- [ ] 6.3 Accept ADR 0027 when the design is confirmed, and make the `define-ui-config-model` task 4.1 note that the GUI now follows this change; verify `docs/adr/README.md` lists it as accepted
- [ ] 6.4 Run fmt, clippy `-D warnings` and the full test suite, then archive this change with `openspec archive add-admin-api --yes`; verify all gates pass and the spec deltas merge

## Workflow follow-up

- Release notes for the next release: a new `[admin]` section and `/admin/v1`; no ledger schema change is expected (revocations reuse the overrides table).
