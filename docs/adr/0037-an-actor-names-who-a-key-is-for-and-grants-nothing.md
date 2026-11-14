# 0037. An actor names who a key is for and grants nothing

- Status: accepted
- Date: 2026-11-12
- Supersedes: none (refines [0025](0025-the-ui-has-no-state-of-its-own.md): it said keys could one day be credentials owned in the database; this says what owns a key and what that ownership means)
- Change: `openspec/changes/add-actors`

## Context

A key is an id and a hash in the configuration file. Statistics group by key, so the usage of one agent that holds
several keys cannot be read together, and nothing says which person an agent acts for. Almost every call arrives from an
agent on behalf of a person. The admin GUI is about to gain usage screens that need an owner to attribute to, and later
changes will want to hang more on that owner: a profile, deny lists, a cap across a person's agents, keys created in the
database (ADR 0025), a sign-in. There is exactly one operator today and no second user to shape those against.

## Decision

A **actor** is a named owner of keys, declared in the configuration file (`[actors.<id>]`), with a required
`kind` (`human` or `agent`) and, for an agent, an optional `owner` that names a human actor. A key names its
actor. A decision records the actor id and the owner id as they were when the request was made, and the owner of
a human is that human, so a person's usage rolls up in one step.

**An actor grants and removes nothing.** `kind` and `owner` are labels. A key's routes, budgets, limits and privileges
come only from the key and its profile, as before (ADR 0011: clients narrow, never widen). Whatever is added to a
actor later may only subtract from or bound what the file grants, never add to it, and a database row never exceeds
the file (ADR 0025). Attribution is by ids recorded on the decision, never by a value the caller supplies in a request.

Alternatives considered:

- **A label string on the key.** Cheapest, but it has no kind and no owner and gives a screen nothing to list.
- **An actor table in the database now.** A second source of truth for one operator, and credentials in a ledger that
  is also a prunable log. Kept for when a second person needs keys without the operator.
- **Authority on the actor from the start** (a profile, deny lists, a key, agent and owner chain). Authority is
  where mistakes become security problems, and there is no user who needs it yet. Added when a person needs a different
  ceiling.
- **Person and agent as separate models.** Two tables for one behaviour; an agent that acts for a person would need a
  link between them anyway.

## Consequences

Usage can be read per agent and per person with no new storage and no change to what any key may do. Later changes
(a profile and deny lists, a cap across a person's agents, database keys, a usage view and sign-in) add to the same
actor without changing what it is or how past decisions read, because the id is stable and the decision keeps the ids
it was made under. We must keep true that no code path reads an actor to authorize, limit or route a request, and that
a value from a request never picks an actor.
