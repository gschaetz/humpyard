# Tasks

## 1. Ledger flush and record on drop

- [x] 1.1 Give the ledger writer a message enum and add `Ledger::flush(&self)` and `Accounting::flush`; unit tests: entries are in the database right after `flush` returns (no polling), flush on a broken database returns without hanging, flush after the channel closed returns
- [x] 1.2 Add `Drop` to `CallContext` completing held entries (calls and cancelled stream templates); tests: a client that disconnects while a classifier route's answer is pending still gets the judge entry recorded, and normal requests record each entry exactly once

## 2. Server lifecycle

- [ ] 2.1 Add `shutdown_grace_secs` to the config (0 to 3600, default 30); config tests for default, valid and out-of-range
- [ ] 2.2 Implement `server::run` with graceful drain, the hard-stop flag, the grace timer, second-signal handling and the bounded flush; integration tests with a signal channel: in-flight request completes after the signal, new connections are refused, a long stream is cut at the grace period and recorded as cancelled, a second signal cuts it at once, and entries are in the ledger when `run` returns
- [ ] 2.3 Wire `main` to OS signals (SIGINT and SIGTERM on Unix) and exit codes; a process-level test starts the binary against a mock upstream, makes a request, sends SIGTERM, and asserts exit status 0 and the entry present in the ledger without waiting

## 3. Docs

- [ ] 3.1 Document shutdown behavior and the config key in `README.md` and `docs/budgets.md`, update ADR-0005's outstanding item by a note in the architecture doc (ADRs are immutable), and update `docs/architecture.md` (flow diagram, status table, module notes); verify the architecture and invariants docs are consistent with the code
