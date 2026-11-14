# ADR Review

## Reviewed

All 37 ADRs in `docs/adr/` were listed and none is superseded. Read for this design:

- ADR-0013 Routing decisions are recorded per request in the ledger database
- ADR-0006 Money is integer micro-USD
- ADR-0027 Admin authority is granted by configuration to named, scoped principals, never to a client key or a database row
- ADR-0036 Admin and UI work must not change the request path's performance

## Outcome

No major durable architectural decision was introduced. The change adds an optional parameter and three fields to a read
that already exists, inside the decisions those ADRs record.

## New ADRs

None.
