# Proposal

## Why

When a model starts misbehaving at 2 a.m., the only ways to stop sending it traffic are a config edit and a redeploy
(a ConfigMap change and an apply here) or waiting for the health breaker to notice. An operator wants to say "stop using
this target for the next two hours" immediately, with no change to the configuration of record, and have it undo itself.
ADR 0025 and `define-ui-config-model` (task 3.1) decided what such an action may be: an expiring, audited override that
only restricts or steers things that already exist. This change builds the first slice of that, and gives `humpyard ctl`
its first command that changes anything.

## Decisions to review (my recommendations)

1. **An override that removes the last eligible target refuses the request (fails closed).** Health fails open ("health
   never denies service"), but an override is an operator's explicit intent, and the usual reason to disable the only
   target is to stop spend or abuse. The refusal says an override caused it, and removing the override restores service.
2. **A separate key setting, `manage_overrides`, gates writing** (default false), distinct from `read_config`. Reading the
   list needs either. The existing operator key stays read-only; you would mint or flag another key to manage overrides.
3. **Three kinds in this slice:** `disable_target`, `drain_provider`, `pin_route`. All only remove candidates. Forcing a
   cooldown and any reordering are left out.
4. **Expiry is mandatory:** 4 hours by default, 24 hours at most (`[overrides]` settings).
5. **State is per instance** (the ledger on this gateway's disk), like health and budgets today.

## What Changes

- Overrides are stored in the ledger, loaded into memory, applied in planning as one more subtract-only input, and expire
  on their own.
- `GET`, `POST` and `DELETE` on `/v1/overrides`, and `humpyard ctl overrides list|add|remove`.
- Every create, revoke and expiry is an audit row; decisions and `explain` show which overrides applied; the effective
  configuration view labels an affected target `override`.
- Metrics: `humpyard_overrides_active{kind}` and the age of the oldest; an alert example for an override that has been
  active too long.
- Out of scope: forcing a cooldown, reordering, multiple replicas sharing overrides, a UI, and key lifecycle (profiles
  and database keys are their own changes).

## Capabilities

### Modified Capabilities
- `gateway-api`: the overrides endpoints.
- `request-routing`: overrides as a planning input.
- `decision-log`: which overrides applied.
- `metrics`: override gauges.
- `cli`: `ctl overrides`.
- `gateway-config`: `[overrides]` and the `manage_overrides` key setting.

## Impact

A ledger table pair, one planning input, three endpoints, one key setting, one config section, two metrics and one CLI
group. A gateway with no overrides behaves exactly as before.
