# ADR Review

## Reviewed

ADR-0004 No plaintext secrets in config, logs or the ledger (the scrub exists to honor it for error text);
ADR-0013 Routing decisions are recorded per request (metadata only, never prompts or responses; this change adds
one bounded provider message per failed attempt); ADR-0026 A request a pool member rejects moves on, within a bound
(it is why several attempts fail in one request); ADR-0023 The 413 and 403 handling (unchanged). The rest of 0001 to
0027 are unaffected.

## Outcome

One durable commitment qualifies: a decision may hold the provider's scrubbed, bounded error message for each
failed attempt, and nothing else derived from a request or response. It refines the "metadata only" line of ADR 0013
without superseding it, and any later field that would hold content needs its own ADR, so it is recorded now.
Tactical choices (where the text is taken, the exact scrub rules, the optional JSON field with no migration) stay in
`design.md`.

## New ADRs

- [ADR-0028 A decision keeps the provider's scrubbed error text for each failed attempt](../../../docs/adr/0028-a-decision-keeps-the-providers-scrubbed-error-text-for-each-failed-attempt.md) (proposed; supersedes none)
