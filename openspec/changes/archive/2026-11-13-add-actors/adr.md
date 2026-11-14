# ADR Review

## Reviewed

All 36 ADRs in `docs/adr/` were listed. None names another in a `Supersedes:` field, so all are in force. Read for this design:

- ADR-0003 Config-first virtual keys behind a `KeyStore` trait
- ADR-0004 No plaintext secrets in config, logs or the ledger
- ADR-0011 Selectors choose the route from request facts; clients narrow, never widen
- ADR-0012 The configuration is an atomically swapped snapshot, reloaded on SIGHUP
- ADR-0013 Routing decisions are recorded per request in the ledger database
- ADR-0025 The UI has no state of its own
- ADR-0027 Admin authority is granted by configuration to named, scoped principals, never to a client key or a database row
- ADR-0035 The admin API's time series are read from the decision log at query time
- ADR-0036 Admin and UI work must not change the request path's performance

## Outcome

One decision qualifies: what an actor is and that it grants nothing. It is a boundary that later changes (profiles
and deny lists on an actor, caps, database keys, sign-in) will build on, and it refines ADR 0025 without superseding
it. The revoke of a client key follows the revoke ADR 0027 already describes and needs no ADR of its own.

## New ADRs

- [ADR-0037 An actor names who a key is for and grants nothing](../../../docs/adr/0037-an-actor-names-who-a-key-is-for-and-grants-nothing.md) (refines ADR-0025)
