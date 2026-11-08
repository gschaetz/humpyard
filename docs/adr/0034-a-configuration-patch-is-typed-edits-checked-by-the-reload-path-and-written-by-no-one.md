# 0034. A configuration patch is typed edits, checked by the reload path, and written by no one

- Status: accepted
- Date: 2026-11-08
- Supersedes: none
- Change: `openspec/changes/archive/2026-11-08-add-config-patch`

## Context

ADR 0025 says a UI change to structural configuration is a patch against the file, validated with the real loader
before it is offered, and that writing the file is an opt-in step conditional on a fingerprint. It does not say
what a patch is made of, who may ask for one, what it may touch, or how "validated" is kept honest. Those choices
decide whether the GUI (4.1), the direct write (5.2) and database keys (5.4) share one safe path or each grow their
own. The file also holds things no interface should edit through a patch: admin credentials and key hashes.

## Decision

A patch is produced from a list of **typed edits** (`set`, `remove`, `add_table`, `remove_table`, each with a path
and a JSON value), never from caller-supplied TOML text. Edits are applied to the file's own text with a
format-preserving editor, so a line no edit touches is unchanged.

- **Allowlist.** Only `providers`, `targets`, `routes`, `profiles`, `aliases`, `budget`, `health`, `overrides` and
  (for `set` and `remove` only, never a hash, never adding or removing a key) `keys` can be edited. `admin`,
  `metrics`, `ledger`, `discovery`, the top-level settings and `select` cannot.
- **One validation path.** The result is checked by the function a reload uses (loader, restart-only checks, the
  generated-file rule), not by a copy of it, so a patch that loads is a patch that reloads.
- **The base is the file, named by its fingerprint.** A request must name the fingerprint of the file's text
  now. A mismatch is refused with the current one. The answer carries the fingerprint the edited text would
  have, which is what a later conditional write (5.2) names.
- **The patch endpoint writes nothing.** It has no write path and no reload trigger. Any later write is a separate,
  opt-in operation that reuses the edits and the fingerprint.
- **Its own scope.** A `config` admin scope, implied by nothing and implying nothing.
- **Nothing secret in the answer.** Key hashes, credential hashes and provider header values are masked (by
  walking the parsed document) before the diff is computed.

Alternatives considered: accept a TOML fragment (no allowlist is possible short of parsing it again, and the
safety argument collapses to trusting a string); JSON Patch on the resolved configuration (loses comments and
file structure); return the whole new file (harder to review, exposes everything); validate with a separate
function (the two drift, and "it loaded in the check" stops meaning anything).

## Consequences

The GUI, the direct write and any future "open a change in the operator's repository" hook sit on one function
whose behaviour is pinned by a spec and tests. A new editable section is a deliberate widening: one line in the
allowlist, its tests, and a threat-model row. The diff is for review, not for `patch(1)` (masked lines do not
apply), so the edit list is the reproducible form. A `config` credential can learn the file's structure by probing
diffs; the scope is separate so a `read` credential cannot, and the residual is documented. We must keep the
reload path and the patch check as one function.
