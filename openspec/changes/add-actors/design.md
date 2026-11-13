# Design

## Context

ADR 0025 (accepted): structural configuration lives in the file, the UI keeps no state of its own, runtime actions are
expiring and audited, and keys may later be credentials in the database, bounded by profiles. Key profiles shipped in
0.26.0. An admin credential can be revoked at runtime by an expiring record kept with the overrides that is not an
override (api.md, "Administration"); a client key cannot. See proposal.md for the motivation and why this step is small.

## Goals / Non-Goals

**Goals:**
- Name the owner of a key, and the person an agent acts for, and attribute usage to both.
- Give a client key a revoke that needs no deploy.
- Leave the model able to take a profile, deny lists, a combined cap or database keys later without changing what a
  actor is.

**Non-Goals:**
- Any authority on an actor. `kind` and `owner` do not widen or narrow what a key may do.
- Database keys, sign-in, OIDC, teams, a request-supplied user tag, agents registered with an endpoint.
- Writing the configuration file.

## Decisions

**1. An actor is declared in the file.** `[actors.<id>]` has a required `kind` (`human` or `agent`), an optional
`display_name`, and for an agent an optional `owner`. A key says `actor = "<id>"`. No new storage, and it is reviewed
like a route. *Alternatives:* a label string on the key (no kind, no owner, nothing for a screen to list); a database
table now (a second source of truth for one operator).

**2. The name is "actor", not "user" and not "principal".** An agent is an actor, so "user" is wrong. "Principal" already means an admin credential in ADR 0027, the admin API and `src/admin.rs`, so reusing it would give two things one name. A registered agent's endpoint, if it ever comes,
goes in a child table keyed by the actor id, not in the actor.

**3. `kind` and `owner` are labels, and the spec says so.** Authority comes only from the key and its profile, so nobody
assumes an agent is treated differently from a person. `kind` is required, with no default, because a default of `human`
would mislabel every existing agent in the statistics. `owner` must name a `human` actor; a human has none and an
agent cannot own an agent, so a rollup is always one step.

**4. Attribution is on the decision, as ids.** A decision records the actor id and the owner id at the time of the
request; changing a key's owner later does not rewrite history. The owner of a human actor is that human, so a
person's own keys and their agents' keys roll up under one owner. The admin statistics and the series gain
groupings by actor and by owner (the client statistics do not, a key sees only its own); a request with none shows under an explicit `(none)`. The ids are what a later cap or
usage view would key on.

**5. A client key is revoked by an expiring record.** The shape of the admin revoke: an admin call, in the scope that
revokes admin credentials, creates an audited record with an expiry no later than the overrides maximum. It can only
remove access, takes effect on the next request, is not an override, is not listed or counted as one, and cannot be ended
through the override endpoints. It lives at `POST` and `DELETE /admin/v1/keys/{id}/revoke`, beside the credentials one. The permanent step is deleting the key from the file and reloading. *Alternative:*
revoking by editing the file (needs a deploy, which is the problem).

**6. The effective configuration shows the actors.** Each with its kind, owner and the ids of its keys, labelled
`file`; secrets and hashes stay omitted.

## Risks / Trade-offs

- [`kind` and `owner` turn out wrong for teams or OIDC] → They commit only to an id, a kind and a one-step owner; teams
  and an external subject are additive.
- [An actor is read as granting something] → Requirement and docs state that it grants and removes nothing, and a
  test shows a key's authority unchanged by its actor.
- [Revoke records pile up] → They expire, are listed with the admin revocations and counted by the existing metric
  family and alert.

## Migration Plan

An additive ledger migration adds the actor and owner columns to decisions. A configuration with no `[actors]`
loads and behaves as before, and rollback is the previous release, which ignores the columns.

## Open Questions

- Whether the query view's group-by control gains `actor` and `owner` in this change or in the GUI change after it.
