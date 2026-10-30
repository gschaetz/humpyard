# Tasks

## 1. The judge

- [x] 1.1 A pure module (registered in `tests/architecture.rs`, depending only on the repair module) that takes what a response showed (text seen, reasoning seen, tool calls with name and accumulated arguments, stop reason or string, whether the stream ended normally, failed or never ended) and returns the four flags; no I/O, `unwrap`, `expect`, panic or fallible indexing; verify table tests for every scenario of the `response-signals` spec: empty, reasoning-only, length stop in each spelling and an unknown stop string, cut-off arguments, blank arguments, a missing name, a non-object, a stream cut before its final chunk, and a good answer with no flag
- [x] 1.2 The bounds: at most 16 tool calls and 64 KiB of arguments per call are judged, and a call past either is left unflagged; verify a test with a 1 MiB argument and with 40 calls, that memory stays within the bound, and that the flags for the calls inside the bound are unaffected

## 2. Observing responses

- [x] 2.1 The stream tap collects what the judge needs from each chunk it already reads (text and reasoning seen, tool-call name and argument fragments per index within the bounds, the stop string, whether a terminal chunk arrived or a failure) and passes every chunk on exactly as before; `submit()` calls the judge when the provider's stream ends or fails, and a dropped tap (client disconnect) leaves `cut_stream` unset; verify tests with a fake streaming upstream for a whole answer, a stream that stops early, a stream that errors, and a client that disconnects mid-stream, and that the bytes the client receives are identical with and without a malformed tool call in the stream
- [x] 2.2 The buffered path judges the aggregated response (content blocks, normalized stop reason) with the same module and sets the flags on the answer's entry, while judge calls and calls with no response to look at stay unknown; verify tests for a buffered empty answer, a buffered length stop and a buffered malformed tool call
- [x] 2.3 Observation cannot fail a request: a response that cannot be judged (no usage, an undecodable chunk) is delivered as today with its flags unknown or unset; verify a test whose upstream sends a decode error in the middle of a stream

## 3. Storing the flags

- [x] 3.1 Ledger schema v13: four nullable integer columns on `usage` (`empty_output`, `truncated_output`, `bad_tool_call`, `cut_stream`), `SCHEMA_VERSION` 13, migrated on the one-connection pool before the real pool opens like v12, and `Entry` carrying them as optional booleans; verify a migration test that opens a v12 file, upgrades it, reads an old row as unknown and writes a new row, and that reopening is harmless
- [x] 3.2 `StreamRecorder::submit()` and the buffered finalize fill the entry's flags from the judge and `Accounting::complete()` queues it as today; verify a real-server test that a served request's usage row holds the right flags for a clean answer, a truncated one and a malformed tool call, and that a judge call's row reads unknown
- [x] 3.3 No content reaches storage: a test where the answer's text, tool name and arguments hold a recognizable string, the call is recorded, and neither the ledger file's bytes, the decisions read through both endpoints, the statistics, the metrics exposition nor the captured logs contain it; verify

## 4. Showing the flags

- [x] 4.1 `GET /v1/decisions`, `GET /admin/v1/decisions` and `humpyard ctl decisions` show the names of the flags set on the serving call (read from the usage row by `request_id` and `kind = 'answer'`) and nothing when none is set or the stream is still open; verify tests on all three for a flagged, a clean, an open-stream and a pre-v13 decision, and that output for a decision without flags is unchanged
- [x] 4.2 `GET /v1/stats` and its admin counterpart add, per group, the number of observed calls and one count per flag, with unknown calls left out of both figures, under the existing window and key-isolation rules; verify tests for a group with a known mix, a window containing pre-v13 rows, and two keys with separate figures
- [ ] 4.3 Update the admin API's OpenAPI contract for the new fields and keep its contract test green; verify the test fails if a documented field is missing from a response

## 5. Metrics

- [ ] 5.1 Add `humpyard_response_calls_observed_total{route,target}` and `humpyard_response_signals_total{route,target,signal}`, counted in `Accounting::complete()` beside the existing usage counters, with `signal` from the fixed set and the existing bounded vocabulary for route and target; verify a test that each flag raises its series, a clean call raises only the observed counter, an unknown target uses the bounded fallback label, and the exposition parses strictly

## 6. Wrap-up

- [ ] 6.1 Document the four signals, their definitions and the schema note: `docs/routing.md` (reading a bad answer), `docs/api.md` (decision and statistics fields), `docs/ctl.md`, `docs/metrics.md` (the two counters) and the one-way schema note and backup advice in `docs/deployment.md`; verify the strict docs build
- [ ] 6.2 Update `docs/architecture.md` (the module table, the response path if it lists the tap, the "Last updated" line and the status table row) and verify the Mermaid diagrams still match the code
- [ ] 6.3 Accept ADR 0030 when the design is confirmed; verify `docs/adr/README.md` lists it as accepted
- [ ] 6.4 Run fmt, clippy `-D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --document-private-items` and the full test suite, then archive this change with `openspec archive record-response-quality-signals --yes`; verify all gates pass and the spec deltas merge

## Workflow follow-up

- Release notes for the release that carries it: each answer is now judged for four content-free facts (empty, cut by the length limit, a malformed tool call, a stream cut early), kept on the usage row and shown on decisions, statistics and metrics; ledger schema v13 is one-way, so back up `ledger.db` (with `-wal` and `-shm`) before upgrading.
- Next changes, in this order: attribute a bad tool call in a later request's history to the target that wrote it; a shadow effective quality shown beside the declared one; applying it to routing with bounds and an off switch.
