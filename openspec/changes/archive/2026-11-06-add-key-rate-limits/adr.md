# ADR Review

## Reviewed

- ADR-0016 Metrics are one registry with bounded labels
- ADR-0017 Strict routes exclude disallowed classes before routing
- ADR-0025 The UI has no state of its own (its key-profile and sharing-gate decisions)
- Invariant 5 (the order of checks before an upstream call) and invariant 7 (a cancelled request is still recorded)

## Outcome

No major durable architectural decision was introduced. A per-key limit in process memory is a small addition to the existing order of checks, using the existing clock, refusal and metric vocabulary; the one structural point (a concurrency permit rides the response body, not the decision guard) is recorded in the design. Invariant 5 in `docs/invariants.md` is updated rather than an ADR written.

## New ADRs

None.
