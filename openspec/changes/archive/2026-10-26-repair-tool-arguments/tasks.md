# Tasks

## 1. The repair

- [x] 1.1 A pure module (registered in the architecture test's layer rules) with one function that takes tool-call argument text and returns `None` when it already parses as a JSON object, otherwise the replacement text: a single pass that tracks string and escape state and the stack of open `{` and `[`, closes an open string (dropping a dangling backslash or partial `\u` escape), drops a trailing comma, colon or valueless key, appends the closers, and falls back to `{}` when the result is not an object; no `unwrap`, `expect`, panic or fallible indexing; verify unit tests for a truncated object, a cut inside a string, inside a nested array, after a colon, after a comma, mid-escape, empty text, non-JSON text, and valid objects with odd spacing returned as `None`
- [x] 1.2 Property-style tests over the function: every prefix of several valid objects yields `None` or a replacement that parses as an object, and a valid object is never rewritten; verify they fail if the fallback or the closer order is broken (mutation check)

## 2. Applying it to a request

- [x] 2.1 Repair the decoded request: every assistant `ToolCall` whose `arguments` is a string that is not an object is replaced by the parsed repair, any other non-object becomes `{}`, an object is untouched; return the number repaired; verify tests for each value kind and that user, system, tool-result and non-tool content is unchanged
- [x] 2.2 Repair each preserved body: a chat body's `messages[].tool_calls[].function.arguments` and a Responses body's `input[]` `function_call` items; a body with only valid arguments stays byte for byte as sent; verify tests for both formats and for a body with no tool calls
- [x] 2.3 Call it in `server.rs` right after the request is decoded, before anything reads the request, and set the count on the decision guard (`Decision.repaired_tool_arguments`, held in memory until group 3 stores it); verify through the real-server tests of 2.4 (the refused-request check moves to 3.2, where the count can be read back)
- [x] 2.4 A real-server test per inbound format: a Responses request and a chat request whose history holds a cut-off call reach a strict fake upstream (it answers 400 for invalid arguments) and succeed, and a request with valid arguments reaches the upstream unchanged; verify it fails without the repair

## 3. Recording and showing the count

- [x] 3.1 Ledger schema v12: `ALTER TABLE decisions ADD COLUMN repaired_tool_arguments INTEGER` (nullable, written only when more than zero), `SCHEMA_VERSION` 12, and a migration test that opens a v11 database, upgrades it and reads an old row with no count; verify reopening is harmless
- [x] 3.2 `repaired_tool_arguments` on the decision record, filled from the guard; verify a round trip and that an old row reads with none; verify a request refused before any provider is tried (an unknown model or a spent budget) still carries the count
- [x] 3.3 `GET /v1/decisions` and `GET /admin/v1/decisions` return the field only when set; `humpyard ctl decisions` prints it only when set; verify tests on all three and that the output for a decision without it is unchanged
- [x] 3.4 A test that no part of the arguments reaches the decision, original or repaired: a request whose cut-off call holds a recognizable string is routed and its stored decision, read through both endpoints, holds the count and none of that string; verify

## 4. Wrap-up

- [x] 4.1 Document the repair and the count in `docs/routing.md` (reading a failed request), `docs/api.md` (the decision field) and `docs/ctl.md` if decisions are described there, plus the one-way schema note and the backup advice in `docs/deployment.md` if upgrades are described there; verify the strict docs build
- [x] 4.2 Update `docs/architecture.md` (the module table, the request flow if it lists the adaptation step, the "Last updated" line and the status table row) and verify the Mermaid diagrams still match the code
- [x] 4.3 Accept ADR 0029 when the design is confirmed; verify `docs/adr/README.md` lists it
- [x] 4.4 Run fmt, clippy `-D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --document-private-items` and the full test suite, then archive this change with `openspec archive repair-tool-arguments --yes`; verify all gates pass and the spec deltas merge

## Workflow follow-up

- Release notes for the release that carries it: tool calls with invalid arguments in a request's history are repaired before routing and counted on the decision; ledger schema v12 is one-way, so back up `ledger.db` (with `-wal` and `-shm`) before upgrading.
