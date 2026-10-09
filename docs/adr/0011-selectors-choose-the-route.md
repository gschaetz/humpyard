# 0011. Selectors choose the route from request facts; clients narrow, never widen

- Status: accepted
- Date: 2026-10-08
- Supersedes: none
- Change: [add-route-selectors](../../openspec/changes/archive/2026-10-08-add-route-selectors)

## Context

The client's `model` names the route, so all of a key's traffic follows one flow. Operators and
customers want several rule sets side by side (for example private, then paid, then public for one
client; paid only for CI) and want the choice to depend on who calls and what the client says about
the request, such as a profile or tags sent by an agent harness. Letting arbitrary client input
choose routes risks escalation to routes a key should not reach.

## Decision

Routing rules are an ordered `[[select]]` table. Each rule has a `when` (all given conditions
hold) and a `route`; the first rule that matches and names a route the key may use decides the
route, and with no applicable rule the requested model is the route (today's behavior).
Conditions cover the requested model, the key id, a standard profile header
(`x-humpyard-profile`), standard tags (`x-humpyard-tag-<name>`), the agent metadata Switchyard
extracts, the stream flag, and any explicitly named header other than credentials. Selection
runs in the authorize stage, before budget and policy, so budgets and eligibility apply to the
chosen route.

The safety rule: **clients can narrow but never widen.** A request can only trigger rules the
operator wrote, and a rule whose route the key may not use (`allowed_routes`) is skipped, never
turned into a failure or an escalation. Credential headers cannot be matched.

Alternatives considered: letting the client name any route through a profile header (bypasses the
operator's mapping and exposes internal route names); an expression language from the start
(harder to review and explain; kept as an escape hatch for later); one rule set per key only
(cannot vary by agent, task or tag); making ineligible selections fail with 403 (lets a header
turn a working request into an error).

## Consequences

Customers can define their own flows without code changes. Rule order matters and shadowing
is possible; an `explain` call and hot reload are the planned follow-ups. The key's
`allowed_routes` becomes the security boundary for client-influenced routing and should be set for
any key whose clients are not fully trusted.
