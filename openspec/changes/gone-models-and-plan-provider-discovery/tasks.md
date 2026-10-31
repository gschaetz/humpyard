# Tasks

## 1. Find out what the provider lists

- [ ] 1.1 Against the live OpenCode Go model list (read through the gateway's own provider key inside the cluster, never printed), record whether it still lists `mimo-v2.5` and whether the dataset and the list agree on the other models in use; write the finding in `design.md` under Open Questions and, if retired models are dropped from the list, add a follow-up task for reporting hand-written targets; verify `design.md` states the finding

## 2. Count unavailable endpoints

- [ ] 2.1 Add `humpyard_endpoint_unavailable_total{provider,target}` to the registry and count each attempt whose class is `unavailable` once, from the same place the failed-attempt facts are recorded, with the bounded vocabulary for both labels; verify tests that a 410, a 404, a 401 and a model-gone 400 each raise the series once, a 429, a timeout and a 5xx do not, an unknown provider or target uses `other`, no error text reaches the exposition, and the exposition parses strictly
- [ ] 2.2 Document the counter in `docs/metrics.md` with the classes it covers and an example alert expression; verify the strict docs build

## 3. Plan providers in configuration

- [ ] 3.1 Add `plan` and `class` to a provider's `discover` (schema, validation, the resolved config), with `class` overriding `[discovery] class` for that provider's generated targets only; verify tests that a provider without them behaves as before, that `class` alone leaves targets priced at zero, and that `plan = true` without `price_source` is rejected naming the provider
- [ ] 3.2 Let the strict loader accept a finite, non-negative price within the hand-written bounds on endpoints of plan providers only, and keep rejecting any priced endpoint elsewhere; verify tests for a valid plan price, a negative, an infinite and a non-numeric price (the whole file ignored), and a priced endpoint on a free provider (the whole file ignored)

## 4. Plan providers in discovery

- [ ] 4.1 In `models sync`, add a model of a plan provider when the provider lists it and the dataset prices it and states a context window, writing the dataset's list price and the provider's class, and remove it when the provider stops listing it; keep every bound (`max_added_per_run`, `max_removed_fraction`, `exclude`, the duplicate rule); verify tests for a new plan model, a model only the provider lists, a model only the dataset prices, an excluded model, the add bound refusing the run (exit 1, nothing written), removal, and that a provider without `plan` still only reports with `check --paid`
- [ ] 4.2 Make `models check` show the price of plan models it reports as new, and keep `--paid` output for providers that are not plan providers; verify a test of the text and JSON output

## 5. Docs, architecture and wrap-up

- [ ] 5.1 Document plan providers in `docs/discovery.md` (the rule, the opt-in, the exclude list for models the account cannot use, and the cluster alert), `docs/configuration.md` and `examples/`; verify the strict docs build
- [ ] 5.2 Update `docs/architecture.md` (the `discover.rs` and `metrics.rs` rows, the status table, the "Last updated" line) and verify its Mermaid diagrams still match the code
- [ ] 5.3 Accept ADR 0032 when the design is confirmed; verify `docs/adr/README.md` lists it as accepted
- [ ] 5.4 Run fmt, clippy `-D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --document-private-items` and the full test suite, then archive this change with `openspec archive gone-models-and-plan-provider-discovery --yes`; verify all gates pass and the spec deltas merge

## Workflow follow-up

- Cluster configuration (separate repository): opt OpenCode Go in with `discover = { strengths = ["general"], plan = true, class = "plan" }` once released, and add a Prometheus alert on `increase(humpyard_endpoint_unavailable_total[1h]) > 0` per provider.
- Release notes: two additions, both opt-in; no migration.
