# ADR Review

## Reviewed

ADR-0004 No plaintext secrets in config, logs or the ledger (the decision keeps a count, never the arguments);
ADR-0013 Routing decisions are recorded per request (metadata only; a count is metadata); ADR-0026 A request a pool
member rejects moves on, within a bound (unchanged; this removes one cause of every member rejecting); ADR-0028 A
decision keeps the provider's scrubbed error text (it is what exposed the cause). The rest of 0001 to 0028 are
unaffected.

## Outcome

One durable commitment qualifies: the gateway rewrites a request's tool-call arguments when, and only when, they
are not a JSON object, and says so on the decision without storing them. It is the first time the gateway edits
conversation content a client sent, so the line it may not cross (valid content untouched, only tool-call
arguments, count only) is recorded now. The scanner's rules, where the repair runs and the schema step stay in
`design.md`.

## New ADRs

- [ADR-0029 The gateway repairs invalid tool-call arguments in a request's history](../../../docs/adr/0029-the-gateway-repairs-invalid-tool-call-arguments-in-a-requests-history.md) (proposed; supersedes none)
