# Proposal

## Why

Every key repeats its own routes, budgets and limits, so keeping ten agent keys alike means editing ten tables and
a drift between them goes unnoticed. ADR 0025 decided that the file defines key *profiles*, named bundles of
authority, and that a key can only narrow its profile. Database-managed keys (task 5.4 of `define-ui-config-model`)
and a per-profile sharing gate (5.5) both need profiles to exist first. This change adds them to the configuration
alone, so they can be reviewed and used before any database key exists.

## What Changes

- A new `[profiles.<id>]` table: `allowed_routes`, `over_budget`, `daily_usd`, `monthly_usd`, `daily_tokens`,
  `monthly_tokens`, `max_request_usd`, `max_requests_per_minute` and `max_concurrent`, each optional.
- A new optional `profile = "<id>"` on a `[keys.<id>]`. A key inherits every one of those settings it leaves
  unset. A setting it does set must be no wider than the profile's, or the configuration is refused naming the key,
  the profile and the setting (the rule ADR 0025 states as `min(key, profile)`).
- A key naming a profile that does not exist is refused. A profile that no key uses is allowed.
- `read_config` and `manage_overrides` stay on the key only: a profile cannot grant a privilege.
- The effective configuration (`GET /v1/config`) shows each key's `profile` and its resolved values, and lists the
  profiles.
- Existing configurations are unaffected: no profile, no change.
- **Not in this change:** a key-level class restriction (`allowed_classes` on a key or profile), a maximum
  lifetime, `max_active_keys` and everything database-managed. Those belong to 5.4, where a key has a lifecycle.

## Capabilities

### New Capabilities
- `key-profiles`: how `[profiles.*]` are defined, how a key inherits and narrows its profile, and what is refused.

### Modified Capabilities
- `gateway-api`: the effective configuration shows each key's profile and the profiles themselves.

## Impact

`src/config` (a raw profile type, resolution of a key against its profile at load, validation), `src/server/config_view.rs`,
`docs/budgets.md` and `docs/configuration.md`, `docs/architecture.md`, `docs/threat-model.md`. No change to the
request path: resolution happens at load, and the limiter, budget tracker and route check read the resolved
`KeyConfig` exactly as now. Reload picks profiles up like any other configuration.
