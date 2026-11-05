# Proposal

## Why

The first run of `humpyard models check --retired` in the cluster (0.23.0, 2026-11-04) found two real retired models and two providers it called "unusable": `codestral`, whose `/v1/models` answers 404 because Mistral's Codestral host has no model list at all, and a local Ollama provider on a laptop that was not reachable on its configured LAN address. Both exit the check with 2, so a weekly job that alerts on a non-zero exit would fail every week for reasons that need no action: one provider can never be asked, the other is a machine that is allowed to sleep or be away by design, with an always-on local provider as the fallback. A check that is always red teaches people to ignore it, and its exit 2 would also hide the one real finding it exists for.

## What Changes

- The check tells apart three ways a provider's list can be missing, instead of one "unusable":
  - **No list**: the provider answers 404, 405 or 501 for `/models`. It has no model list to compare with, so it is reported as skipped and does not affect the exit code.
  - **Unreachable**: the connection fails or times out, or the answer is 502, 503 or 504 (what a proxy or mesh says for a dead upstream). The check could not ask, so it says so, and it does not affect the exit code; whether a provider is up is the health checks' and the new unavailable counter's job.
  - **Unusable**: the provider answered but the answer cannot be used (a rejected key, any other error status, a body that is not a model list, an empty list). This stays exit 2: it means the check is incomplete for a reason someone should look at.
- The text report and the JSON report gain these categories (`no_list` and `unreachable`, each with the provider and the reason); existing JSON fields keep their meaning.
- No configuration change, no migration, no new ADR.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `cli`: `models check --retired`: the three ways a list can be missing and what each does to the report and the exit code.

## Impact

- `src/discover.rs` (the provider-list fetch returns what kind of failure it was) and `src/retired.rs` (the report and exit code); discovery's own behavior is unchanged, it keeps treating any failure as "no list".
- `docs/discovery.md`, `docs/architecture.md`.
- Cluster side (not this repo): the weekly job that runs the check.
- Not in scope: a per-provider switch to exclude a provider from the check, and any change to how discovery reads lists.
