# Design

## Decisions

- **The tail is a guard on candidates, not a second route.** Planning builds one ordered candidate list: pool
  members (speed or quality order), then tail targets. Each tail candidate carries the set of failure classes
  that may reach it. Switchyard still falls through the list; humpyard's existing failure classification decides
  whether a failed attempt may move to a tail candidate (its class must be in `then.on`) or only to the next
  pool member.
- **Why not widen `fallback_on`.** It applies to the whole route, so `["overflow"]` would also forbid
  member-to-member failover. A per-tail guard keeps the pool's own rules and gates only the paid step.
- **Overflow is decided before the call where possible.** A prompt larger than every member's declared window
  is subtracted from the pool by planning (ADR 0019); the tail is then the only candidate, so the paid target
  serves it without a wasted attempt.

## Risks

- A tail target of class `paid` on a route a key may use widens that key's spend. Validation requires the
  tail's classes to be allowed by the route's `allowed_classes` when one is set, and `explain` names the tail.
