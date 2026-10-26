# Tasks

## 1. The scrub

- [x] 1.1 A pure `scrub` function (new module, registered in the architecture test's layer rules): one line, control characters and line breaks to single spaces, `Bearer` tokens, known key prefixes, `key=`/`api_key=`/`token=`/`access_token=` values and long mixed letter-and-digit runs replaced with `[redacted]`, cut to `MESSAGE_LIMIT` characters on a character boundary; verify unit tests for each rule, a multi-line input, a multi-byte character at the cut, and ordinary text left alone
- [x] 1.2 A function that reads a provider's error text from a body: the JSON `error.message` (including a body wrapped in a list), otherwise the start of the body, capped before scrubbing; verify tests for an OpenAI-shaped body, a Google-shaped list body, plain text, an empty body and a body that is not UTF-8-clean
- [x] 1.3 Route a decision's own `message` through the same scrub as well as the cut; verify the existing decision tests pass and a decision whose error text holds a key-shaped string stores it redacted

## 2. Recording the message

- [x] 2.1 `message` on the pool's attempt report and on the stored `Attempt` (optional, skipped when absent, defaulting when a row lacks it); verify a round-trip test and that a row written without the field still reads
- [x] 2.2 Fill it in the pool from the error it already holds: upstream HTTP answers (status and text), timeouts and connection failures (their description), nothing for a success; verify pool tests for a rejection, a rate limit, a timeout and a success
- [x] 2.3 A failover test through the real server: three providers reject one request and the decision lists three attempts, each with its own provider's message; verify it fails if the message is dropped from any attempt (mutation check)
- [x] 2.4 A test that no request content reaches the decision: a request whose body holds a recognizable string is rejected by a provider that does not echo it, and the string appears nowhere in the stored decision; verify

## 3. Showing it

- [ ] 3.1 `GET /v1/decisions` and `GET /admin/v1/decisions` return the field with each attempt; verify a test on both, and that a row without it returns attempts unchanged
- [ ] 3.2 `humpyard ctl decisions` prints the message with an attempt when present; verify a ctl test, and that the output for a decision with no messages is unchanged
- [ ] 3.3 Document the field and its scrubbing in `docs/api.md` (decisions) and `docs/routing.md` (reading a failed request), and the retention note in `docs/budgets.md` if decisions are described there; verify the strict docs build

## 4. Wrap-up

- [ ] 4.1 Update `docs/architecture.md` (the module table and the "Last updated" line; the status table row) and verify the Mermaid diagrams still match the code
- [ ] 4.2 Accept ADR 0028 when the design is confirmed; verify `docs/adr/README.md` lists it
- [ ] 4.3 Run fmt, clippy `-D warnings` and the full test suite, then archive this change with `openspec archive record-attempt-errors --yes`; verify all gates pass and the spec delta merges

## Workflow follow-up

- Release notes for the release that carries it: failed attempts now show the provider's scrubbed message; no ledger schema change and no migration.
