# Design

## Context

See proposal.md for why. What the code does today:

- A Codex request arrives as a Responses request. The translation crate (`switchyard-translation` 0.3.0) decodes a
  `function_call` item's `arguments` as it is: a JSON string stays a `Value::String` holding the client's text,
  valid or not. Encoding for a chat provider writes that text verbatim (`json_string`), so a cut-off string reaches
  the provider as it is. A chat request is decoded differently (`parse_arguments` turns unparsable text into
  `{"raw": ...}`), but the same-format body the client sent is kept in `preservation.requests` and forwarded as the
  client wrote it, so a chat client's bad arguments also pass through.
- `pool.rs` already adapts a request for chat providers (`adapt_for_chat_providers`: the `developer` role and
  Responses-shaped reasoning details) once per call, and `add_tool_call_extra_content` already rebuilds the
  preserved chat body when an endpoint needs it. The decision's request fields (`stream`, `prompt_tokens`,
  `tools`, `images`) are filled in `server.rs` right after the request is decoded.
- Decisions are a table with one column per field; a new field has meant an additive, nullable column and a
  `user_version` bump each time (v7 to v11). The attempts are the exception, a JSON list.
- In-force ADRs that bound this: 0004 (nothing secret or request-derived in logs or the ledger), 0013 (a decision
  never contains prompts or responses) and 0028 (the one narrow exception: a provider's scrubbed message).

## Goals / Non-Goals

**Goals:** one cut-off tool call in a client's history cannot make a provider refuse every later turn; a valid
call is never altered; the work is linear in the request and cannot fail the request; the decision says it
happened without keeping any of the text.

**Non-Goals:** stopping a model from emitting a cut-off call (that is the response side, and a streamed answer
cannot be taken back); repairing anything but assistant tool-call arguments; guessing the meaning of lost text;
a per-provider switch; changing pool membership (a cluster-config matter).

## Decisions

### 1. Repair once, in the server, right after the request is decoded

The repair runs where the request's features are computed, before planning, so it happens once per request rather
than once per attempt, applies to every route and provider, and the count is on the decision guard without any
plumbing through the pool. It edits the decoded request and every preserved body.

*Alternative:* inside `adapt_for_chat_providers` in the pool. Rejected: it runs per attempt (each of up to a dozen
endpoints), and the count would have to travel back through `call_detailed` and the attempt callback.
*Alternative:* a per-provider `repair_tool_arguments` switch like `drop_fields`. Rejected: every provider benefits,
the strict ones are the ones that matter, and a switch somebody forgets reproduces the stuck session.

### 2. What counts as valid, and how a bad value is repaired

A pure function takes the argument text and returns `None` when it already parses as a JSON object, otherwise the
replacement text. Replacement is: trim; then complete the text by scanning once while tracking whether it is inside
a string (and an unfinished escape) and the stack of open `{` and `[`; at the end close an open string (dropping a
dangling backslash or partial `\u` escape), drop a trailing comma, colon or a key that has no value, and append the
closers in reverse order. If that parses as an object it is the result, otherwise `{}`. Empty text, text that is
not JSON, and JSON that is not an object (`[]`, `"x"`, `3`, `null`) all become `{}`.

The scan is one pass over the text, so the work is linear and bounded by the request size the server already
limits; there is no recursion and no allocation beyond the output. It cannot panic: it uses no indexing that can
fail and no unwrap.

*Alternative:* always `{}`. Rejected: when the object was only cut at the end the first keys are usually intact and
give the model its own context back; `{}` is the fallback, not the rule.
*Alternative:* an external "JSON repair" crate. Rejected: a new dependency for forty lines whose behaviour we need
to pin in tests.

### 3. Where the arguments live

- The decoded request: `ContentBlock::ToolCall.arguments` on assistant messages. A `Value::String` is repaired and
  replaced by the parsed object; a `Value::Object` is valid and untouched; any other value becomes `{}`.
- Each preserved body: a chat body's `messages[].tool_calls[].function.arguments` and a Responses body's
  `input[]` items of type `function_call` (`arguments`). A string is replaced by the repaired text; a value that is
  already a valid object string is not rewritten, so a valid body stays byte for byte the client's.

The count is the larger of the number of decoded calls repaired and the number repaired in a preserved body. They
describe the same calls and must not be added, and neither alone is enough: the Responses decoder keeps the bad
text as a string (so the decoded side sees it), while the chat decoder turns bad text into `{"raw": ...}`, which is
already an object (so only the preserved chat body, the one that is replayed, still shows it).

### 4. The decision keeps a count, in an additive column

`repaired_tool_arguments` is a nullable integer on `decisions`, written only when it is more than zero and shown
by the decisions endpoints and `ctl decisions` only then. Schema v12 is an `ALTER TABLE ... ADD COLUMN`, the same
kind as v7 to v11: an old row reads as no count, and it cannot be undone, so the upgrade needs the same ledger
backup as 0.18.0.

*Alternative:* put the count in the attempts JSON. Rejected: it is a property of the request, not of an attempt,
and a refused request has no attempt at all.

### 5. Documented as a behaviour, not hidden

`docs/routing.md` (reading a failed request) and `docs/api.md` (the decision field) say what is repaired and what
is not, so the gateway's one deliberate rewrite of client history is written down.

## Risks / Trade-offs

- **The gateway rewrites part of what the client sent** → only where it was invalid for every strict provider, never
  when valid, and counted on the decision. ADR 0029 states the line.
- **A completed call is a guess** → it is history, not an instruction; the call already ran with whatever the model
  produced, and the alternative is a session no strict provider can serve.
- **Lenient providers lose nothing; strict providers gain** → no provider sees a request it accepted before
  changed in a way that matters, because valid arguments are untouched.
- **A model keeps emitting cut-off calls** → not fixed here; the count makes it visible, and which model it was is
  readable from the preceding decisions.
- **A hand-written scanner has edge cases** → it is pure and gets property-style tests (every prefix of a valid
  object ends in a valid object; a valid object is returned unchanged) plus the named cases.
- **One-way schema step** → additive and nullable; roll back by restoring the pre-upgrade ledger copy.

## Migration Plan

Back up `ledger.db`, `-wal` and `-shm` before the upgrade (the cluster procedure used for 0.18.0), then replace the
binary; the column is added on first start. Rolling back past this release means restoring the backup, because the
ledger version cannot go down. Nothing changes for a client; a client that was stuck continues on its next turn.
