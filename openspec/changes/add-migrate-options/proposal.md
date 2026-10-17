# Proposal

## Why

Migrating a real ModelRelay setup into an existing humpyard configuration showed two things the generated
fragment cannot express. ModelRelay's `opencode` provider is OpenCode Zen (the free tier), while the target
configuration already has a provider called `opencode` (the Go plan), so the fragment collides with it and
its environment variable name. And the generated pool routes are speed-ordered; to prefer capable models the
operator then edits every route by hand to add a `quality_weight`. Both are one-line options on the command.

## What Changes

- **`--rename FROM=TO`** (repeatable) names a ModelRelay provider something else in the fragment. The new name
  is used for the provider table, its generated environment variable (`<TO>_API_KEY`) and the prefix of its
  targets. `TO` must be lowercase letters, digits and dashes, and must not repeat another generated provider.
- **`--quality-weight W`** (0 to 1) writes `quality_weight = W` on every generated `fastest` route, so the pools
  prefer models with a declared quality (ADR 0020, amended) without hand edits.
- Out of scope: renaming routes or targets individually, other route settings.

## Capabilities

### Modified Capabilities
- `cli`: two options on `migrate-modelrelay`.

## Impact

`src/migrate.rs` (naming, route rendering), `src/main.rs` (flags), docs (`migrating-from-modelrelay.md`). No
config or API change.
