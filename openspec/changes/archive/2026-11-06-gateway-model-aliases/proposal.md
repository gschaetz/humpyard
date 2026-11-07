# Proposal

## Why

Claude Code can fill its `/model` picker from a gateway's `GET /v1/models`
(`CLAUDE_CODE_ENABLE_GATEWAY_MODEL_DISCOVERY=1`, as of 2.1.311), but it keeps only ids that match
`/claude|anthropic/i`. humpyard lists real route and target names (`deep-thinker-public`,
`coding-paid`, ...), so discovery finds nothing and a client has to hard-code each pool in a wrapper
script. Letting an operator publish a name that Claude Code accepts, with a label and a description,
makes every pool pickable at runtime with no client-side lists.

## What Changes

- New optional `[aliases.<id>]` configuration table: `model` (a route or target), `display_name`,
  `description`.
- `GET /v1/models` lists each alias the key may use, next to the real names, with `display_name` and
  `description` when set.
- A request whose `model` is an alias is resolved to the real name once, at the edge, before any
  other stage looks at it. Old names keep working unchanged.
- Key `allowed_routes` stay real names; a key may use an alias exactly when it may use the alias's
  target.
- A top-level `model_aliases` switch (default `true`); `false` stops resolving and listing aliases
  without removing the table.
- Config load rejects an alias that collides with a route, target or other alias, points at nothing,
  or has an invalid shape.

Not changing: routing, budgets, policy, the ledger schema, the order of the request stages.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `gateway-api`: `GET /v1/models` lists aliases with display fields; requests may name an alias.
- `gateway-config`: the `[aliases]` table and its validation.

## Impact

- `src/config/schema.rs`, `src/config/validate.rs`, `src/config/mod.rs`: parse, validate, expose.
- `src/server.rs`: `list_models` and one resolution step where the requested model is read.
- `docs/api.md`, `docs/configuration.md`, `docs/clients.md` (Claude Code discovery recipe),
  `docs/architecture.md`, `docs/threat-model.md`.
- No new dependencies. Existing configurations load unchanged.
