# Tasks

## 1. Count unavailable endpoints

- [x] 1.1 Add `humpyard_endpoint_unavailable_total{provider,target}` to the registry and count each attempt whose class is `unavailable` once, from the same place the failed-attempt facts are recorded, with the bounded vocabulary for both labels; verify tests that a 410, a 404, a 401 and a model-gone 400 each raise the series once, a 429, a timeout and a 5xx do not, an unknown provider or target uses `other`, no error text reaches the exposition, and the exposition parses strictly
- [x] 1.2 Document the counter in `docs/metrics.md` with the classes it covers and an example alert expression; verify the strict docs build

## 2. Report endpoints the provider no longer lists

- [ ] 2.1 Fetch a provider's `/models` list for the check the way discovery does (key to that provider's host only, size and time limits, defensive parsing), reusing the discovery fetch code where it fits; verify tests with a fake provider for a valid list, a non-list body, an oversized body, a rejected key and a timeout
- [ ] 2.2 Add `models check --retired` with `--provider` and `--json`: compare every configured endpoint with its provider's list, report the ones not listed, exit 1 with a report, 0 with none and 2 when a list could not be used while still checking the other providers; verify tests for a retired model, all listed, one unusable provider among usable ones (exit 2, others checked), a provider with no `discover` or `price_source`, and that no file is written
- [ ] 2.3 Document `--retired` in `docs/discovery.md` and the CLI reference, including that a provider may serve a model it does not list, and a suggested weekly job; verify the strict docs build

## 3. Wrap-up

- [ ] 3.1 Update `docs/architecture.md` (the `metrics.rs` and `discover.rs` rows, the status table, the "Last updated" line) and verify its Mermaid diagrams still match the code
- [ ] 3.2 Run fmt, clippy `-D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --document-private-items` and the full test suite, then archive this change with `openspec archive report-retired-model-endpoints --yes`; verify all gates pass and the spec deltas merge

## Workflow follow-up

- Cluster configuration (separate repository): a Prometheus alert on `increase(humpyard_endpoint_unavailable_total[1h]) > 0` per provider, and a weekly job running `humpyard models check --retired` beside the price check.
- Release notes: two additions, both opt-in (the counter exists always; the check runs only when invoked); no migration.
