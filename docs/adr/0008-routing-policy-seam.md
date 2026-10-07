# 0008. A routing policy seam in front of Switchyard's algorithms

- Status: accepted
- Date: 2026-10-07 (recorded retroactively; decided 2026-10-06)
- Supersedes: none
- Change: [add-switchyard-routing](../../openspec/changes/archive/2026-10-06-add-switchyard-routing), [add-cost-tracking](../../openspec/changes/archive/2026-10-07-add-cost-tracking)

## Context

Budget state (and later provider health) must influence routing, but Switchyard's algorithms accept
no such input. A spike showed they read their targets from a per-request `RuntimeModels` grouped by
category, that `Category::Any` must list every target, and that an emptied tier is an error rather
than a degradation.

## Decision

Before each request the gateway asks a `RoutingPolicy` which targets are eligible for this caller
(`is_eligible(context, target)`), and algorithms only ever see eligible targets. Ineligible targets
are removed from every group, `Any` is rebuilt, an emptied tier is served by the surviving tier
(capable and efficient substitute for each other; a judge falls back to a remaining tier), random
weights are re-aligned, and HTTP 503 is returned only when nothing is eligible. Policies compose (a
target must satisfy all of them); the budget policy is the first, health-aware policy is next. The
policy context uses gateway-owned types, not Switchyard's.

Alternatives considered: wrapping or replacing Switchyard's algorithms (loses upstream behavior and
session state); filtering the algorithm's outcome afterwards (fallbacks and judge calls would still
reach ineligible targets).

## Consequences

- Algorithms stay stock Switchyard, and budget or health logic lives in one replaceable place.
- Substitution silently changes which tier serves a request; it is logged and visible through the
  `x-conductor-target` header.
- A target's price for budget rules is the highest across its endpoints, so policy decisions are
  conservative.
- The seam is ordered before routing (invariant 6 in [invariants.md](../invariants.md)).
