# ADR Review

## Reviewed

ADR-0005 (SQLite usage ledger with an asynchronous writer), in force. Its consequences list
graceful shutdown as outstanding work; this change completes that item and does not alter the
decision. ADR-0001 to ADR-0004 and ADR-0006 to ADR-0008 were scanned and are unaffected.

## Outcome

No major durable architectural decision was introduced. The shutdown design implements the
consequences ADR-0005 already accepted; the mechanics (watch flag, flush marker, record on drop)
are tactical and recorded in design.md.

## New ADRs

None.
