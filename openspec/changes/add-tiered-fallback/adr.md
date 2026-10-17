# ADR Review

## Reviewed

ADR 0016 (pools), ADR 0019 (capabilities and window subtraction), ADR 0020 (ordering), ADR 0023 (unavailable
endpoints and fallback classes). None is contradicted; the tail reuses the fallback classes of ADR 0023.

## Outcome

A new ADR records the guarded tail: why a per-tail guard rather than a route-wide `fallback_on`.
