# Tasks

## 1. Classify why a provider's list is missing

- [x] 1.1 Make the provider-list fetch say what kind of failure it was (no list: 404, 405, 501; unreachable: connect error, timeout, 502, 503, 504; unusable: everything else including a rejected key, other error statuses, a body that is not a model list, an oversized body and an empty list), with `fetch_provider_models` mapping every kind to the existing error so discovery's behavior and messages do not change; verify tests for each status and failure above against a mock provider, that a refused connection and a time limit are unreachable, and that the discovery tests still pass unchanged
- [x] 1.2 Report no-list and unreachable providers in `retired` (text lines and the JSON fields `no_list` and `unreachable`, each with provider and reason) without changing the exit code, keeping exit 2 for unusable providers and exit 1 for a retired endpoint, and checking the other providers in every case; verify tests for a provider answering 404 (skipped, exit 0), one that refuses connections (unreachable, exit 0), an unreachable provider beside a retired endpoint elsewhere (exit 1), an unusable provider (exit 2), and the JSON shape

## 2. Docs and wrap-up

- [x] 2.1 Describe the three cases and the exit codes in `docs/discovery.md` (replacing the "unusable" paragraph) and update `docs/architecture.md`; verify the strict docs build
- [x] 2.2 Run fmt, clippy `-D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --document-private-items` and the full test suite, then archive this change with `openspec archive tolerate-providers-without-a-model-list --yes`; verify all gates pass and the spec delta merges

## Workflow follow-up

- Cluster configuration (separate repository): the weekly job that runs `humpyard models check --retired` can alert on a non-zero exit once this is released; until then the two providers above make it exit 2.
- Release notes: the exit code of `models check --retired` changes for providers with no list or that are unreachable (0.23.0 shipped yesterday); no configuration change.
