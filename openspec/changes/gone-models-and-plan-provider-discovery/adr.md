# ADR Review

## Reviewed

- ADR-0004 and ADR-0013 (no content is stored)
- ADR-0009 Endpoint health lives in the pool and fails open
- ADR-0014 Prices are proposed, never fetched at runtime
- ADR-0015 Provider quota is polled, advisory and fails open
- ADR-0022 Free-pool membership may be updated on a schedule without review
- ADR-0023 Answers that describe the endpoint make it unavailable
- ADR-0030 Response quality is recorded as content-free flags per call (proposed; unrelated)

## Outcome

One durable decision qualifies: discovery may add models for a flat-fee plan provider, with the price and class the design describes. It refines ADR 0022 without superseding it, since free pools and pay-per-token providers are unchanged. The unavailable-endpoint counter follows ADR 0023's classes and is not a decision of its own.

## New ADRs

- [ADR-0032 A flat-fee plan provider may be discovered, and unavailable endpoints are counted](../../../docs/adr/0032-a-plan-provider-may-be-discovered-and-unavailable-endpoints-are-counted.md) (refines ADR-0022)
