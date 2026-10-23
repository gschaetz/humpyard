# Design

## The three kinds

| Kind | Subject | Effect |
|---|---|---|
| `disable_target` | a target name | the target is removed from every request's candidates |
| `drain_provider` | a provider name | every target endpoint of that provider is removed (a target with endpoints on other providers keeps those) |
| `pin_route` | a route name and a target name | the route's candidates are cut to that one target |

Each is created with a reason and an expiry (`expires_in_secs`, defaulting to `[overrides] default_secs`, 14400, and
at most `max_secs`, 86400). All three only remove candidates.

## What an override may never do

- Name something that is not in the loaded configuration (a target, provider or route that does not exist): refused at creation.
- `pin_route` to a target that is not already one of that route's candidates (listed targets, or the pool's resolved
  members), or whose class a strict route forbids: refused. A pin therefore never adds reach.
- Create or change a target, provider, price or key, or widen what a key may use. There is no such kind.

An override that names something a later reload removes stays stored but has no effect, and `GET` marks it `stale`.

## Where it applies

Planning already takes a per-target eligibility check (`Route::plan_ranked`'s `eligible`). Active overrides add to it,
evaluated at request time against the clock, so an override ends at its expiry with no background job needed. Order: class
strictness, capabilities, then overrides, then health (an overridden target is simply not a candidate, so health and the
guarded tail see the reduced set). `pin_route` for a pool route cuts the pool to the pinned member and leaves a guarded
tail as configured.

## When nothing is left

If overrides remove every candidate, the request is refused with HTTP 503 and the error code `override_no_target`
(`explain` shows it). This is deliberate and differs from health, which fails open: removing an override restores service
at once. The decision records the overrides that applied.

## Storage and expiry

Two ledger tables: `overrides` (id, kind, subject, route, reason, created_ms, expires_ms, created_by, revoked_ms) and
`override_audit` (id, override_id, event `created|revoked|expired`, at_ms, by). Active overrides are loaded at startup
and kept in memory; the API updates both. An expiry writes its audit row when first noticed (the next request, or the next
scrape or `GET`). The audit table is pruned with the decisions (`decision_retention_days`); the overrides table keeps
active rows and recent history. A gateway without a `[ledger]` refuses to create an override (409), because nothing would
survive a restart.

## Who may use it

- Create and remove: a key with `manage_overrides = true`. Anything else gets 403 `overrides_not_manageable`.
- List: a key with `manage_overrides` or `read_config`.
- Each audit row names the key id that acted, never the key.

## Visibility

- Each decision lists the overrides that applied (id, kind, subject).
- `explain` marks removed targets with the override id as the reason.
- `GET /v1/config` labels a disabled target `override` in its source.
- `humpyard_overrides_active{kind}` and `humpyard_override_oldest_age_seconds`; the alert rules include an example,
  `HumpyardOverrideLong`, firing when the oldest active override is over six hours old.

## ctl

`humpyard ctl overrides list [--history]`; `add --disable-target T | --drain-provider P | --pin ROUTE=TARGET --reason TEXT
[--for 2h]`; `remove ID`. `add` prints what it created and when it expires. Exit codes follow the existing rules (403 is a refusal, 1).

## Not here

Reordering and forced cooldowns, sharing overrides across replicas (the ledger is per instance, as health is), and a UI.
