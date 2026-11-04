# 0029. The gateway repairs invalid tool-call arguments in a request's history

- Status: accepted
- Date: 2026-10-26
- Supersedes: none
- Change: `openspec/changes/repair-tool-arguments`

## Context

A client that talks to the gateway keeps the conversation and sends all of it on every turn. When a model ends a
turn with a tool call whose JSON arguments are cut off, the client stores that call and replays it. Providers
differ in what they accept: NVIDIA and Google refuse a call whose arguments are not a JSON object, while several
free providers accept it. On 2026-10-26 a Codex session was refused twice in a row by NVIDIA (`messages[15].tool_calls[0].function.arguments
must be a valid JSON object string`) because of one such call from an earlier turn, and the same shape very likely
caused the stuck turn of 2026-10-25. Nothing the session does afterwards can fix its history, so every strict
provider refuses every later turn and only a lenient pool, or a new session, gets through. The gateway sits between
the client and the providers and sees the value on every request.

## Decision

Before a request is routed, the gateway makes every assistant tool call's arguments a JSON object. A call that
already is one is left exactly as the client wrote it. A truncated object is completed when that yields an object,
and anything else (not JSON, empty, or JSON of another kind) becomes `{}`. Nothing else in the request is changed.
This is the only place the gateway rewrites what a client sent as conversation content, and it does so only where
the content was invalid for providers that check it.

The decision records how many calls were repaired and never the arguments, original or repaired, so it adds no
request content to the ledger (ADR 0004, ADR 0013).

Alternatives considered: refusing the request with a clear error (the session is still stuck, now by us);
failing over only to lenient providers (hides the problem, narrows the pool, and a lenient provider may still
choke later); a per-provider switch (the strict providers are the ones that matter, and a forgotten switch
reproduces the stuck session); doing nothing and telling people to start a new session (loses the conversation).

## Consequences

A conversation with one cut-off call keeps working on every provider. The model sees, in its history, a call that
ran with partial or empty arguments, which is closer to what happened than a request nobody can serve. The
gateway now owns a small rewrite of client history, so the rule to keep true is narrow: valid arguments are never
touched, only tool-call arguments are ever rewritten, and a repair is counted on the decision and never stored.
The decision table gains a column (ledger v12), an additive and one-way step. This does not stop a model from
producing a cut-off call; that remains a question of which models serve a pool.
