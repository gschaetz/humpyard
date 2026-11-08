# Design

## Context

ADR 0025 puts structural configuration in the file and makes every other interface a producer of patches. The
pieces that exist: `Config::from_toml_with(text, env, read)` is the one loader; `try_reload` in `src/server.rs`
runs it, then `restart_only_changes`, then builds the snapshot; `watched_fingerprint(text, generated)` is the
fingerprint a reload compares, and `GET /admin/v1/config` reports it. What does not exist: any way to change the
file's text without rewriting it (the `toml` crate drops comments and ordering), and a handle on the file's path
from a request handler (only the reload loop holds it).

## Goals / Non-Goals

**Goals:**
- Produce a diff for a typed change, that the real loader has already accepted, with comments and ordering kept.
- Refuse a stale base, and say what the current fingerprint is.
- Touch nothing: no file write, no reload, no ledger row.
- Make the same check a reload will make, from the same code, so "the patch loads" and "the reload loads" cannot
  drift apart.

**Non-Goals:**
- Writing the file (5.2), key creation or revocation (5.4), a GUI, or editing the generated discovery file.
- Editing `select` rules (an array of anonymous tables: positions shift under edits; deferred until there is a
  need), or `admin`, `metrics`, `ledger`, `discovery` and the top-level settings.
- Making the diff applicable with `patch(1)` in every case (see "The diff is for review").

## Decisions

### One endpoint, typed edits

`POST /admin/v1/config/patch` with `{"fingerprint": "...", "edits": [...]}`. An edit is
`{"op": "set", "path": ["targets", "groq-x", "endpoints", 0, "max_request_tokens"], "value": 8000}`,
`{"op": "remove", "path": [...]}`, `{"op": "add_table", "path": ["targets", "new"], "value": {...}}` or
`{"op": "remove_table", "path": [...]}`. A path is table names, keys and array positions; a value is JSON mapped to
TOML (string, integer, float, boolean, array, and object as an inline table).

Alternatives: *free-form TOML from the caller* (rejected: it bypasses the section allowlist and can say anything
the parser accepts, so the safety argument becomes "trust the string"); *JSON Patch against the JSON view of the
configuration* (rejected: the JSON view is the resolved configuration, and writing it back would lose comments and
cannot express file structure); *returning the whole new file* (rejected: larger, harder to review, and exposes
every line instead of the changed ones).

### Edits are applied to the file's text with `toml_edit`

The gateway reads the file at request time and parses it with `toml_edit::DocumentMut`, which keeps comments,
ordering and whitespace for everything an edit does not touch. `set` replaces a value in place and keeps the
decor around it. `add_table` appends a standard table after the last existing table. `remove_table` removes the
table and its leading comment. Keys and values go through the library's API, never string concatenation, so a
name with a quote or a newline cannot become syntax; path segments are also limited to a conservative character
set (letters, digits, `-`, `_`, `.`, `:`, `+`, `/`), widened only when a real name needs it.

### The check is the reload's check

Today `try_reload` reads, loads, compares restart-only settings and builds. That sequence moves into one function,
`validate_text(text, running)`, used by `try_reload` and by the patch. The patch uses the same environment lookup
and generated-file reader as a reload, and applies `restart_only_changes` against the running configuration, so a
change a reload would refuse is refused here. One more rule: the loader drops an invalid generated file and loads
the main text alone (ADR 0022). A reload accepts that. A patch does not: if the running configuration merged the
generated file and the patched one would not, the patch is refused, because that is almost never what the editor
meant.

### Fingerprint and base

The base is the file's text now, and the fingerprint is `watched_fingerprint(text, generated)` over that text with
the running configuration's generated file. The request must name that fingerprint. Because the base is the file
and not the running snapshot, an edit made between a file change and its reload is judged against the file, and a
caller holding the running configuration's fingerprint is told the file has moved. The new fingerprint is the
same function over the edited text, so a later direct write (5.2) can condition on it.

### The diff is for review

A unified diff with three lines of context, from the `similar` crate, over the before and after text. Before
diffing, both texts are passed through the same masking step that replaces every key hash, credential hash and
provider header value with `<redacted>`, found by walking the parsed document, not by pattern matching on lines.
Nothing secret reaches a response, and removing a provider shows its header lines without their values. The cost:
a diff that crosses a masked line does not apply with `patch(1)` there. The edit list the caller sent is the
reproducible form, and the operator's pipeline (a template, a ConfigMap) usually applies a change by hand anyway.

### The server learns the file's path

`AppState` gains the path the reload loop already has (`Option<PathBuf>`, `None` when the gateway was started
without a file it can re-read). `None`, or an unreadable file, is 503 `config_file_unavailable`.

### Limits

Body 64 KiB, 64 edits, path depth 8, value nesting 4, checked before the file is read or any edit applied. The
file is the operator's and is not bounded here.

### Scope

A new `AdminScope::Config`, implied by nothing and implying nothing. An editor needs `read` as well, to learn the
fingerprint; the two are separate so a viewer never gets the ability to ask the gateway what a change would look
like. The scope is in `keygen`, the OpenAPI document and `docs/api.md`.

### Observability

One info log line per request: the admin id, number of edits and outcome (`ok`, `unchanged`, `stale`,
`invalid_edit`, `rejected`). Never the values or the file.

## Threat model

Assets: the configuration file's text (routes, key policy, provider secrets' names), admin and key hashes, and the
integrity of what the operator applies. Actors: an admin holding `config`; one holding only `read`; someone who
steals a `config` credential; the operator editing the file by hand at the same time; a GUI built on this (4.1).
Trust boundary: B3, operator to gateway through the admin API; and B4, the configuration file.

| # | Threat | Category | Mitigation | Proof |
|---|---|---|---|---|
| 1 | An edit mints access or rewrites authority: a new admin credential, a changed key hash, a new key | Elevation | Only named sections are editable; `admin` is not; under `keys` only `set`/`remove`, never `sha256`, never adding or removing a key; a rejected edit applies nothing | spec scenarios; unit tests per forbidden path |
| 2 | A response shows a key hash, a credential hash or a provider header value | Information disclosure | Both texts are masked by walking the parsed document before diffing; no other field carries file content | test removing a provider with headers; test over every section with a hash |
| 3 | A stale base overwrites someone's newer change | Tampering | The request names the fingerprint of the file now; a mismatch is 409 with the current one; the new fingerprint is returned for a later conditional write | spec scenarios; test editing the file between read and request |
| 4 | A patch loads here but the reload then refuses it | Tampering | One shared validation function, including restart-only checks and the generated-file rule | test: each restart-only setting is refused by both paths |
| 5 | Crafted keys or values inject TOML syntax | Tampering | Edits go through the editor's API; segments use a restricted character set; the result is re-parsed by the loader | tests with quotes, newlines and brackets in names and values |
| 6 | An oversized body, a long edit list or deep nesting exhausts memory or CPU | Denial of service | Limits are checked before the file is read; the credential is an authenticated admin | boundary tests at each limit |
| 7 | The endpoint writes to disk, or triggers a reload | Tampering | The handler has no write path and no reload trigger; every outcome leaves the file, ledger and reload status unchanged | test comparing file bytes, fingerprint and reload status after each outcome |
| 8 | A `config` credential is used to read the file's structure by probing diffs | Information disclosure | **Gap, accepted:** the diff's context shows lines near an edit (secrets masked). `config` is a separate scope so a `read` credential cannot do this; give it only to someone who may see the structure | residual, documented in `docs/api.md` and the threat model |
| 9 | Logs record the file or the edit values | Information disclosure | Only the id, counts and outcome are logged | test on the log line's fields |

## Risks / Trade-offs

- **A new dependency pair** (`toml_edit`, `similar`) → both are widely used and small; `cargo deny` must pass, and
  `toml_edit` shares maintainers with `toml`, which the gateway already uses.
- **The editor can keep formatting but not fix it** → a new table is added in the house style (standard table, one
  inline `endpoints` array where the file already uses one); a pathological hand layout is edited in place and
  never reflowed.
- **The loader falls back silently when the generated file is bad** → the patch refuses in that case (above), so
  a reload and a patch can differ only in being stricter here.
- **`select` and the top-level settings cannot be edited** → by design for this change; a later change can add
  them with a path form for array positions and its own threat rows.
- **Environment differences**: validation reads the gateway's own environment, so a variable that exists at patch
  time but not at reload would hide a failure → the same lookup the reload uses, and a reload is a restart-free
  operation that keeps the old configuration on failure.
