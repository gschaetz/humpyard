# ADR Review

## Reviewed

- ADR-0025 The UI has no state of its own (the profile model: a key narrows its profile, `min(key, profile)`)
- ADR-0012 The configuration is an atomically swapped snapshot, reloaded on SIGHUP (profiles resolve inside the load, so a reload stays all or nothing)
- ADR-0016 Metrics are one registry with bounded labels (no new metric)

## Outcome

No major durable architectural decision was introduced. The profile model is ADR 0025's; this change implements its configuration half. The one structural choice, resolving a key against its profile at load so the rest of the gateway reads only the resolved key, follows from ADR 0012 and is recorded in the design.
