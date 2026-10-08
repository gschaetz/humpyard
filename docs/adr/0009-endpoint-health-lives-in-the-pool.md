# 0009. Endpoint health lives in the pool and fails open

- Status: accepted
- Date: 2026-10-08
- Supersedes: none
- Change: [add-endpoint-health](../../openspec/changes/archive/2026-10-07-add-endpoint-health)

## Context

Failover is stateless: with a dead provider first in a target, every request waits out its
timeout before the next endpoint runs (3.6 s per request in the 2026-10-08 real-network test).
ADR-0008 anticipated health feeding the routing policy.

## Decision

Each endpoint (provider + model) carries an in-memory circuit breaker inside the pool: after
`failure_threshold` consecutive failover-class failures it is skipped for an exponentially growing
cooldown, then re-admitted by a single probe. The same classification that triggers failover
(`fails_over`) decides what counts. If every endpoint of a target is cooling down, the target is
still tried, so health only reorders and skips and never denies service.

Alternatives considered: a health-aware `RoutingPolicy` that makes cold targets ineligible (rejected
for now: a policy cannot know whether a route has a healthy alternative, so it could turn slowness
into 503; it needs route topology and a fall-back step, deferred); persisting health in the ledger
database (state is seconds-lived and re-learned within a few requests; persistence adds writes for
no benefit); a background prober (spends tokens and quota on idle providers).

## Consequences

Dead endpoints cost one probe per cooldown instead of one timeout per request. State is lost on
restart and is per process. Tier substitution does not yet react to health. The pool, not the
policy, owns the breaker, so `health` is a core module with a clock and config dependency only.
