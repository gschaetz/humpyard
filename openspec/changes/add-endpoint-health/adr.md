# ADR Review

## Reviewed

ADR-0008 (routing policy seam) names health-aware policy as the next policy. This change
deliberately does not use the seam yet, for the fail-open reason in design.md; ADR-0008 stays in
force and unchanged. ADR-0002 (dispatch on Switchyard's client) is unaffected: the breaker wraps
the endpoint walk inside our own target client.

## Outcome

A durable decision: health state lives in the pool, per endpoint, in memory, fail-open.

## New ADRs

- 0009 Endpoint health lives in the pool and fails open.
