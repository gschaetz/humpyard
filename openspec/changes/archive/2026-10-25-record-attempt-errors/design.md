# Design

## Context

See proposal.md for why. What the code does today:

- A decision stores its attempts as a JSON list in one column (`attempts`); an `Attempt` has the target, provider,
  model, an outcome class, the time taken, the target's class and an optional retry time. Reading a row ignores
  fields it does not know and fills missing ones with defaults.
- The pool already holds the provider's answer when it reports an attempt (`LlmClientError::UpstreamHttp` carries
  the status and the body), but reports only the outcome class.
- A decision's own `message` is the last error's text cut to 200 characters (`MESSAGE_LIMIT`); it is not scrubbed.
  With failover that is the final provider's text only, which is how three rejections left one message.
- In-force ADRs that bound this: 0004 (no keys or hashes in logs or the ledger), 0013 (a decision never contains
  prompts or responses, rows are pruned by retention), 0026 (a rejection moves a pool on, which is why several
  attempts now fail in one request).

## Goals / Non-Goals

**Goals:** every failed attempt says what its provider said; nothing key-shaped or multi-line is stored; no schema
change; no new dependency; no effect on routing, health, fallback or cost.

**Non-Goals:** storing request bodies or full response bodies; a debug switch that logs upstream bodies; showing
messages in `explain` (it is a dry run and makes no upstream calls); changing how errors reach a client; metrics.

## Decisions

### 1. The message is taken where the pool already has the error

The pool reports each attempt with the outcome class. It also reports a `message` built from the same error: for
an upstream HTTP answer, the status and the provider's error text; for a timeout or a connection failure, the
error's own description. Nothing new is fetched and no body is read twice. The text is the JSON `error.message`
when the body has one (including a body wrapped in a list, as Google's is), otherwise the start of the body.

*Alternative:* recording the whole body. Rejected: bodies can be large and can echo request content, and the
message is what a person needs.

### 2. One scrub, used for attempts and for the decision's own message

A small pure function turns provider text into a single line of at most `MESSAGE_LIMIT` characters: control
characters and line breaks become single spaces, and these are replaced with `[redacted]`: a `Bearer` token, a
value with a known key prefix (`sk-`, `sk_`, `gsk_`, `nvapi-`, `AIza`, `ghp_`, `xox`), the value of a `key=`,
`api_key=`, `token=` or `access_token=` query parameter, and any unbroken run of 32 or more letters, digits, `-`
or `_` that mixes letters and digits. The last rule also hides long ids a message mentions, which is the safe
direction to be wrong in. The decision's own `message` goes through the same function, so there is one rule for
error text.

*Alternative:* a list of provider-specific patterns. Rejected: it rots, and a key shape the list misses leaks.

### 3. No schema change

`message` is an optional field on `Attempt`, stored in the existing JSON column and skipped when absent. A row
written before this change reads with no message, and an older binary reading a newer row ignores the field, so
there is no migration, no `user_version` bump and no one-way step. Rollback is replacing the binary.

*Alternative:* a column per attempt row. Rejected: attempts are not rows, and a column would force the one-way
migration for no gain.

### 4. Shown wherever attempts already are

`GET /v1/decisions` and `GET /admin/v1/decisions` return attempts as stored, so the field appears with no handler
change; `humpyard ctl decisions` prints it with the attempt when present.

## Risks / Trade-offs

- **A provider's error can echo a fragment of the request** (for example the offending value) → the 200-character
  cut, the single-line form and the scrub bound it, ADR 0028 states it plainly, and a key reads only its own
  decisions, so the fragment is the caller's own text. An admin credential reads every key's decisions, which is
  already the case for their `message`.
- **The scrub can hide a harmless long id** → accepted; the status and the leading words remain.
- **Rows grow** → at most 200 characters per failed attempt, pruned with the decision.
- **A message that is not text** (a binary or empty body) → no message is stored and the outcome class remains.

## Migration Plan

None. The field is optional on write and on read. Deploy by replacing the binary; roll back the same way.
