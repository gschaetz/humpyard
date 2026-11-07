# 0033. Model aliases are resolved once at the edge and never reach authorization

- Status: accepted
- Date: 2026-11-06
- Supersedes: none
- Change: `openspec/changes/gateway-model-aliases`

## Context

Clients choose models by name, and some clients filter the names they will show or accept (Claude Code's
gateway model discovery keeps only ids containing `claude` or `anthropic`). humpyard's names are the
operator's own (`deep-thinker-public`), and renaming pools to satisfy one client would change every key
allowlist, selector rule and script that mentions them. Key grants (ADR 0003) and selector rules
(ADR 0011) are written against route and target names, and the request stages run in a fixed order.

## Decision

An operator may publish additional names (aliases) for a route or target, each with an optional display
name and description. An alias is resolved to the real name once, when the requested model is read, before
authorization or selection. Every later stage, and every record the gateway keeps, sees only the real name.
Key allowlists name real names only, and an alias is usable by a key exactly when its target is. Aliases
cannot point at aliases and cannot collide with a route, target or another alias.

Alternatives considered: rename the pools (breaks every existing reference); resolve in each consumer
(spreads alias awareness over authorization, selectors and policy); let allowlists name aliases (two names
for one grant, harder to audit); generate `claude-` aliases automatically (implicit naming, no labels).

## Consequences

Adding presentation names for a client costs one config table and no routing change. The rule to keep
true: nothing after the decode stage may depend on, store or compare an alias. Statistics and the ledger
cannot show which alias was used; if that is ever wanted it is its own decision.
