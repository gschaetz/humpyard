# ADR Review

## Reviewed

ADR 0017 (strict routes), 0018 (health-driven eligibility), 0019 (capabilities), 0021 (pools), 0023 (answers that
describe the endpoint) and its amendments (410/402, 413 as overflow), 0024 (guarded tail), 0025.

## Outcome

New ADR 0026, [A request a pool member rejects moves on, within a bound](../../../../docs/adr/0026-a-request-a-pool-member-rejects-moves-on-within-a-bound.md).
It refines ADR 0023 for 400 and 422 without superseding it: the endpoint-describing set is unchanged, the rule that a
400 ends the request still holds for routes that do not opt in, and the 413 amendment is its precedent for a class that
does not count against health.
