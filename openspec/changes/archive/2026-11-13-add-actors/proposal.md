# Proposal

## Why

Nothing in the gateway knows who a key belongs to. Statistics group by key, so the usage of one agent that holds several
keys cannot be read together, and nothing says which person an agent acts for. A leaked client key can only be retired by
a review and a deploy, where an admin credential can be revoked at once. The admin GUI has a query view and is about to
gain usage screens, which need an owner to attribute usage to. Today the callers are the operator's own agents, so this
change stays small: name the owners in the file, attribute usage to them, and give a client key the revoke an admin
credential already has. It grants and removes no authority; that stays with keys and their profiles.

## What Changes

- A **actor** is a named owner of keys, declared in the configuration file as `[actors.<id>]` with a required
  `kind` of `human` or `agent`, an optional `display_name`, and, for an agent, an optional `owner` naming a human
  actor (the person it acts for). A key names its actor with `actor = "<id>"`; a key without one behaves
  exactly as before.
- The loader refuses a key that names an actor that does not exist, an `owner` that is not a human actor, and an
  owner on a human.
- A decision records its actor and that actor's owner (a human's own keys count as that human's). The admin
  statistics and series can group by actor and by owner; the client statistics do not, since a key sees only its own.
- `GET /admin/v1/config` and the effective configuration show each actor and the ids of its keys; `GET /v1/key/info`
  shows a key's actor and owner.
- A client key can be **revoked without a deploy**, the way an admin credential can: an audited, expiring record that
  only removes access, endable only by an admin and not through the override endpoints. Deleting the key from the file
  remains the permanent step.
- **Actors grant and restrict nothing.** `kind` and `owner` are labels: a key's routes, budgets and limits come from
  the key and its own profile, as today.

## Capabilities

### New Capabilities
- `actors`: the actor in the configuration (kind, owner), the checks on it, and attribution on decisions.

### Modified Capabilities
- `client-auth`: key info shows the actor; a revoked key is refused like an unknown one.
- `decision-log`: a decision records its actor and owner.
- `routing-statistics`: admin statistics and the series group by actor and by owner.
- `admin-api`: client-key revoke endpoint, and revoked keys shown in the effective configuration.

## Impact

- **Code:** the `[actors]` table and its checks in `config/`, an actor and owner column on decisions (ledger
  migration), the groupings in statistics and the series, a revoke endpoint and its record, the OpenAPI contract and
  `docs/api.md`.
- **Request path:** a key's actor and owner are resolved at load, so authentication does no extra work; the revoke
  check reads the same in-memory set an admin revocation does. The change still carries a fitness-function entry and an
  isolation measurement (ADR 0036).
- **Security:** one new admin action (revoking a client key) under the scope that revokes admin credentials; the threat
  model gains a note.
- **GUI:** none here; the usage screens are their own changes on the API this one extends.
- **Configuration:** nothing is required of existing deployments. A configuration with no `[actors]` loads and
  behaves as before.
- **Not in this change, and what it leaves ready:** a profile or deny lists on an actor, a cap across an actor's
  keys, keys created in the database, an actor's own usage view and sign-in, teams, and agents registered with an
  endpoint. An actor id is stable and is what decisions record, so each of these adds to the model without changing
  what an actor is or how history reads.
