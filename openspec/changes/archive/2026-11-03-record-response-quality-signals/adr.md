# ADR Review

## Reviewed

ADR-0004 No plaintext secrets in config, logs or the ledger (flags only; tool names and arguments are inspected in memory and dropped); ADR-0013 Routing decisions are recorded per request (metadata only; the decision shows flag names read from the usage row); ADR-0016 Metrics use fixed label values (two new counters, route and target labels from the existing vocabulary, signal from a fixed set); ADR-0028 A decision keeps the provider's scrubbed error text (unchanged; the flags add no text); ADR-0029 The gateway repairs invalid tool-call arguments (the `bad_tool_call` test reuses its definition of invalid; it is the only rewrite of client content and this change adds none). The rest of 0001 to 0029 are unaffected.

## Outcome

One durable commitment qualifies: response quality is recorded as content-free facts per call, kept on the usage row, and they never decide routing by themselves; turning them into a quality value is staged (shown, replayed, then applied within bounds with an off switch). It sets the boundary for the next two changes, so it is recorded now. The signal definitions, the bounds on accumulation and the schema step stay in `design.md`.

## New ADRs

- [ADR-0030 Response quality is recorded as content-free flags per call](../../../docs/adr/0030-response-quality-is-recorded-as-content-free-flags-per-call.md) (proposed; supersedes none)
