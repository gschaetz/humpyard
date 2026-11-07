# Tasks

## 1. Configuration

- [x] 1.1 Add `max_requests_per_minute` and `max_concurrent` to `[keys.<id>]` (positive integers, the first at most 100000, zero and non-integers rejected naming the key and the setting) and show them in the effective configuration; verify config tests for a valid pair, zero, negative, a non-integer and a value over the maximum, and the effective-configuration output

## 2. The limiter and the request path

- [x] 2.1 Add a limiter module with an exact sliding window (a ring buffer of admission times, `Retry-After` from the oldest entry) and a concurrency permit released by `Drop`, driven by the gateway's `Clock`, one entry per configured key, saturating arithmetic; register the module in the architecture rules; verify unit tests for exactly N admitted per 60 s and the N+1th refused, the window sliding, `Retry-After`, independent keys, the permit returned on drop, a clock moving backwards, and state not growing with traffic
- [x] 2.2 Check the rate limit in the request path after the route is authorised and before the budget, for the endpoints that reach a provider only; add a refusal with status 429, `Retry-After` and an OpenAI-style `rate_limit_exceeded` body, record it as a decision with the outcome `rate_limited`, add `rate_limited` to the refusal vocabulary, and make sure it cannot start failover or touch endpoint health; verify integration tests that the N+1th request is 429 and makes no upstream call, `Retry-After` is right under a test clock, a slid window admits again, the budget is not consumed, the decision and `humpyard_refusals_total{reason="rate_limited"}` appear, another key is unaffected, a failed authentication changes no counter, and `/v1/stats` is not limited
- [x] 2.3 Take a concurrency permit at admission and carry it in the response body so it is released when a buffered response is sent, when a stream ends or fails, and when the client disconnects; refuse the N+1th concurrent request with 429 and `Retry-After: 1`; verify tests with a slow fake upstream that an open stream holds the permit and the next request is 429, finishing the stream frees it, a client disconnect frees it, an upstream error frees it, and the in-flight count returns to zero after a burst
- [x] 2.4 Apply a reload to the limits (a changed or removed limit applies from then on, counters for keys no longer configured are dropped, permits in flight are kept); verify a reload that raises a limit admits the next request, one that removes a key drops its state, and an in-flight stream keeps its permit across a reload

## 3. Docs and wrap-up

- [x] 3.1 Document the limits in `docs/configuration.md` and `docs/api.md` (the 429, its body and `Retry-After`), `docs/deployment.md` (one limiter per replica, reset on restart, and that a weak operator-chosen secret is the operator's to avoid: use `keygen`), update invariant 5 in `docs/invariants.md`, and close the rate-limit gap in `docs/threat-model.md`; verify the strict docs build
- [x] 3.2 Update `docs/architecture.md` (the module table, the status table, the "Last updated" line, the request-flow diagram's order of checks) and verify its Mermaid diagrams still match the code
- [x] 3.3 Check the threat model in `design.md` against what was built (each row still true, each proof exists) and record the result in the pull request
- [ ] 3.4 Run fmt, clippy `-D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --document-private-items` and the full test suite, then archive this change with `openspec archive add-key-rate-limits --yes`; verify all gates pass and the spec deltas merge

## Workflow follow-up

- Release notes: new, off by default; existing configurations are unaffected.
- Later, each its own change: limits in key profiles (ADR 0025), a limit shared by several keys, tokens-per-minute limits.
