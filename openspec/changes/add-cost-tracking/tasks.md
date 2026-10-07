# Tasks

## 1. Configuration and pricing

- [x] 1.1 Add `[keys.*]`, `[ledger]`, `[budget]` and endpoint `price` to the config with validation (hashes well formed, limits non-negative, `restricted_at` in 0 to 1, budget needs a ledger, USD budgets need every endpoint priced); unit tests cover each gateway-config scenario
- [x] 1.2 Implement the cost calculator in integer micro-USD from `Usage` and endpoint prices (input, cached input, output); unit tests include the $0.003 example and cached-token handling

## 2. Client authentication

- [x] 2.1 Define the async `KeyStore` trait and the config-backed implementation keyed by SHA-256 hash; unit tests cover lookup and unknown keys
- [x] 2.2 Authenticate inference endpoints via `Authorization: Bearer` or `x-api-key`, 401 in each endpoint's error shape, open mode without keys, route allowlist with 403; integration tests cover all client-auth scenarios and that no upstream call happens on 401/403
- [x] 2.3 Add `keygen <id>` and verify the printed hash authenticates the printed key; add a test that logs contain neither key nor hash

## 3. Usage ledger

- [x] 3.1 Create the SQLite schema with `user_version` migrations and the asynchronous batching writer (bounded channel, failure counting); tests cover insert, batching and an unavailable database not failing requests
- [x] 3.2 Implement startup hydration query for the current UTC day and month per key; test spend after a simulated restart

## 4. Usage capture

- [x] 4.1 Wrap the route's target clients per request in a metered client with a call context; the inner target client reports the serving endpoint; test that attribution matches the served endpoint after failover
- [x] 4.2 Record buffered calls and mark the returned call as answer and the others as judge; tests with a judge route assert two entries charged to the same key
- [x] 4.3 Tap streamed responses: record at the terminal event, and on drop with the missing-usage marker; tests cover normal end, no-usage stream and client disconnect in all three protocols
- [x] 4.4 Record entries for failed runs as judge spend; test a classifier route whose answer call fails

## 5. Budget enforcement

- [x] 5.1 Implement the budget tracker (daily and monthly counters, injectable clock, lazy rollover, hydrated from the ledger); unit tests cover rollover at UTC midnight and month end
- [x] 5.2 Enforce exhausted budgets: 402 before any upstream call in each endpoint's error shape; `free_only` mode serves zero-priced targets; tests cover both and the no-free-target 402
- [x] 5.3 Extend `PolicyContext` with the key id and budget state and implement `BudgetPolicy` (restricted ceiling, free-only, conservative target price); tests assert tier substitution near the limit
- [x] 5.4 Add `GET /v1/key/info`; test it reflects spend immediately after a request

## 6. Docs and integration

- [x] 6.1 Document keys, prices, budgets, `keygen`, the overshoot bound and the managed-keys migration path in `docs/budgets.md` and README; verify the documented config loads and a documented request flow works against mock providers
- [x] 6.2 Update `docs/architecture.md` (status table, request-flow diagram with auth, metering and the ledger writer, module list) and verify against the code
- [x] 6.3 End-to-end test: a key spends past its daily limit through an escalating route, is restricted, then blocked with 402; after a restart its spend is recovered from the ledger
