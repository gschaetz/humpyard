# ADR Review

## Reviewed

- ADR-0004 and ADR-0013 (no content is stored)
- ADR-0016 Metrics are one registry with bounded labels
- ADR-0017 Strict routes
- ADR-0025 The UI has no state of its own
- ADR-0027 Admin authority is granted by configuration to named, scoped principals
- ADR-0030 Response quality is recorded as content-free flags per call

## Outcome

One durable decision qualifies: the admin GUI runs as its own process from the same binary (`humpyard ui`), and the gateway serves no GUI. It refines ADR 0025 (which left hosting open and called the GUI a separate client) without superseding it: the GUI still has no state of its own and still uses only the published admin API; this settles where it runs and what protects it.

## New ADRs

- [ADR-0032 The admin GUI runs as its own process from the same binary](../../../docs/adr/0032-the-admin-gui-runs-as-its-own-process-from-the-same-binary.md) (refines ADR-0025)
