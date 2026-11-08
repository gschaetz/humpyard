# ADR Review

## Reviewed

- ADR-0019 Declared capabilities remove targets that cannot serve a request (this change adds one more declared capability on the same rule)
- ADR-0016 Metrics are one registry with bounded labels (one more bounded reason)
- ADR-0021 Strengths build pools at load time and never remove a target (the pool filter keeps reading `context_window`)

## Outcome

No major durable architectural decision was introduced. A per-request cap is one more declared capability under ADR 0019's rule (remove only on certainty, only subtract, record and count), with its own reason in an already bounded vocabulary. The choice of a separate field over reusing `context_window` is recorded in the design.
