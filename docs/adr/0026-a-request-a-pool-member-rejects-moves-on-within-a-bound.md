# 0026. A request a pool member rejects moves on, within a bound

- Status: proposed
- Date: 2026-10-24
- Supersedes: none (refines ADR 0023)
- Change: `openspec/changes/fail-over-on-rejected-requests`

## Context

ADR 0023 split upstream answers: a 400 or 422 describes the request, so it ends the request and counts for nothing;
a 404, 401, 403, 402, 410 or a model-gone 400 describes the endpoint. That is right when the request is malformed
and wrong when it is the endpoint that cannot take it. Codex through a free pool on 2026-10-23 was broken four times
by members that rejected a field or turn shape the other members accept (Google AI, Groq, NVIDIA, Codestral). Each
time healthy members sat behind the strict one and the route failed until an operator replayed the request to find
the field. A 400 is a statement about the request *relative to one provider*, and a pool is exactly the place where
other providers can answer.

## Decision

A 400 or 422 that is not a model-gone answer has the fallback class `rejected`. On `fastest` and pool routes it moves
the request to the next endpoint and target by default; elsewhere it is opt-in through `fallback_on`, so a private
tier is never silently retried on a public provider. It does not count against the endpoint (like the 413 in the
2026-10-19 amendment to ADR 0023) and it is bounded: three different providers rejecting the same request end it, and
the client gets the last rejection. The guarded tail (ADR 0024) is reached only by classes its `on` list names.

Alternatives considered: widening `unavailable` (it counts against health, and a strict member would cool and then be
probed again for a fault it does not have); falling through on every 400 with no bound (a malformed request would
walk the whole pool); parsing the error text to learn which field to drop and remembering it per provider (every
provider words it differently, and it would be hidden state); a default `drop_fields` list (a guess about providers
that goes stale).

## Consequences

One strict member no longer takes down a route; the cost is one cheap rejected round trip per request it rejects,
which `drop_fields` removes where it is worth removing. A malformed request on a pool route costs up to three
rejected calls. ADR 0023's rule that a 400 ends the request still holds for routes that do not opt in. The
decision log and metrics show `rejected` attempts, so a persistent mismatch is visible instead of silent.
