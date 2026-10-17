# 0024. A pool route can end in a guarded tail

- Status: accepted
- Date: 2026-10-17
- Supersedes: none
- Change: `openspec/changes/add-tiered-fallback`

## Context

A free-model pool is large and unreliable, so its members fail over among themselves freely (ADR 0021,
ADR 0023). The tiers a gateway replaces often mean something more specific: "the free pool, and only when a
prompt is too large for every free model, a paid one". A pool route could not name extra targets, and the
route-wide `fallback_on = ["overflow"]` would also stop one free member failing over to another, which is the
point of a pool.

## Decision

A `fastest` route with a `pool` may have `then = { targets = [...], on = [...] }`. The planned order is the
members, ordered as before, then the tail targets in written order. Members keep the route's own fallback
rules. Only the **last member** of a request's order is held to `then.on` (default `overflow`): a failure of a
class not listed ends the request there instead of reaching the tail. A prompt larger than every member's
declared window removes the members in planning (ADR 0019), so the tail serves it with no wasted attempt. When
every member is merely cold (ADR 0018), they are tried again before the tail is used, so health skipping never
turns into paid spend.

Tail targets are ordinary targets: classes, capabilities and health apply to them, and a tail target whose
class a strict route forbids is refused at load. `explain` lists the tail after the members with its guard.

## Consequences

- One paid target can be the safety net for a whole free pool without widening what a failure may spend.
- The guard sits on one attempt, the last member's, because Switchyard's fallthrough is a flat ordered list;
  humpyard already decides per target which failures may continue (`Metered`), so no new mechanism was needed.
- A tail only exists on pool routes; other route kinds keep their explicit target lists.

## Alternatives considered

- Widening `fallback_on` per route: rejected, it also gates member-to-member failover.
- Two routes, the second used by the client on error: rejected, it moves the policy into every client.
