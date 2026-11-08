# Proposal

## Why

ADR 0025 says the admin GUI keeps no state: a structural change (a target, a route, a price, a key's policy) is
a reviewable patch to the configuration file, never a change made behind the file's back. Nothing produces that
patch yet. Today an operator edits TOML by hand and finds out at reload time whether it loads, and any tool that
tried to edit the file would have to rewrite it and lose the comments and ordering that make it readable. Task 5.1
of `define-ui-config-model` is the piece that makes the rest possible: the GUI (4.1), the direct-write option
(5.2) and key management (5.4) all sit on a function that turns "what I want changed" into a patch the real loader
has already accepted.

## What Changes

- A new admin endpoint `POST /admin/v1/config/patch` and a new admin scope `config`. The body carries the
  fingerprint the editor last read (from `GET /admin/v1/config`) and a list of typed edits.
- The edit kinds are `set` (a value at a path), `remove` (a key), `add_table` and `remove_table` (a target, route,
  profile and so on). Edits are structured data, never free-form TOML text, so a caller cannot produce syntax the
  loader has not seen, and they are limited to named configuration sections (the `[admin]` table, key hashes and
  provider secrets are not editable this way).
- The gateway applies the edits to the text of its configuration file with a format-preserving TOML editor, so
  comments, ordering and spacing survive, then loads the result with the same loader, environment and generated
  file as a reload, including the checks a reload makes (a change that needs a restart is refused). Only a result
  that loads is offered.
- The answer is a unified diff and the fingerprint the new text would have. A result the loader rejects returns the
  loader's reason and no patch. A request whose fingerprint is no longer the file's is refused with the current
  fingerprint.
- **Nothing is written, ever, by this change.** The file, the running configuration and the ledger are untouched.
  Writing the patch is task 5.2 and stays opt-in.
- Limits keep the endpoint cheap: a request-body cap, a maximum number of edits and a maximum path depth.
- **Not in this change:** the optional direct write (5.2), database-managed keys (5.4), a GUI, editing the
  generated discovery file, and editing across more than one file.

## Capabilities

### New Capabilities
- `config-patch`: turning a list of typed edits into a validated patch against the configuration file, without
  writing it.

### Modified Capabilities
- `admin-api`: `config` joins the scopes an admin credential may hold, and the endpoint that needs it.

## Impact

- Code: a new `src/config/patch.rs` (edit application and validation), an admin handler in `src/server/admin.rs`,
  `AdminScope::Config` in `src/config/mod.rs` and the keygen and OpenAPI paths that list scopes, and the server
  state gaining the configuration file's path.
- Dependencies: `toml_edit` (format-preserving editing) and a small diff crate. Both must pass `cargo deny`.
- Docs: `docs/api.md`, `docs/configuration.md`, `docs/architecture.md`, `docs/threat-model.md`, `docs/roadmap.md`,
  and a new ADR.
- Existing configurations and clients are unaffected: a credential without the new scope cannot call the endpoint,
  and no existing endpoint changes.
