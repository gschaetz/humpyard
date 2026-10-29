# Tasks

## 1. Find out what the library surfaces

- [x] 1.1 Check, with a test against a fake upstream, whether the Switchyard client turns a provider's SSE keep-alive comments into events (the design's keep-alive risk) and that the library's call returns only after a streamed answer's first event; record the result in `design.md` and, if keep-alives do surface, make the idle guard count only events that carry output; verify the test passes and `design.md` states the finding

## 2. The wait to start

- [x] 2.1 Build each endpoint's backend with no library deadline (`timeout: None`) and wrap `endpoint.client.call(...)` in `call_detailed` in a `tokio::time::timeout(timeout_secs, ...)` that yields an `LlmClientError::Timeout` built the way the library builds it, so classification as `timeout`, failover, endpoint health, the attempt record and the 504 are unchanged; verify tests that a provider that never answers fails over to the next endpoint within the limit (streamed and buffered), that the attempt reads `timeout`, and that a provider that answers in time is not affected

## 3. The idle limit on a started stream

- [x] 3.1 Wrap a streamed response in a guard that waits for each next event with `tokio::time::timeout(timeout_secs, ...)`, yields one timeout error item when it elapses and ends, resets on every event, and passes events through untouched and in order; verify tests with a fake streaming upstream for a stream that runs longer than `timeout_secs` in total with chunks arriving in time (completes with every event), a stream that goes silent mid-answer (ends with the timeout error and the call is recorded as failed), and a slow start followed by a steady stream (not ended when the original wait would have run out); use a 1 s limit so the tests stay fast
- [x] 3.2 The three messages: `no response started within N ms, retries included`, `response did not finish within N ms, retries included` (buffered) and `stream was idle for N ms`, with the failure class `timeout` in every case; verify tests that each situation produces its message, that the recorded attempt and the `stream iteration failed` log line carry it, and that the client's error event for an idle stream is the usual upstream-timeout error

## 4. Buffered answers keep their limit

- [x] 4.1 A non-streamed call that does not finish within `timeout_secs` fails with a timeout and fails over as today; verify a test with a slow buffered upstream, and that the whole-call limit includes the provider's retries

## 5. Docs and architecture

- [ ] 5.1 Describe `timeout_secs` as the three limits in `docs/configuration.md`, `docs/routing.md` (failover on a timeout) and the comment in `examples/config.toml`; verify the strict docs build
- [x] 5.2 Update `docs/architecture.md` (the `pool.rs` row, the status table, the "Last updated" line) and verify its Mermaid diagrams still match the code
- [x] 5.3 Accept ADR 0031 when the design is confirmed; verify `docs/adr/README.md` lists it as accepted
- [ ] 5.4 Run fmt, clippy `-D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --document-private-items` and the full test suite, then archive this change with `openspec archive bound-the-wait-for-a-response-not-the-whole-stream --yes`; verify all gates pass and the spec delta merges

## Workflow follow-up

- Release notes: a streamed answer that runs longer than `timeout_secs` with chunks arriving now completes (behavior change); no configuration change and no migration.
- Operators: a `timeout_secs` chosen to fail over quickly (such as 45) can stay as it is; it no longer cuts a long healthy stream, so there is no value to change.
