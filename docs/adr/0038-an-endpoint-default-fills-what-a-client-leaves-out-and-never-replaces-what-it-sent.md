# 0038. An endpoint default fills what a client leaves out and never replaces what it sent

- Status: accepted
- Date: 2026-11-14
- Supersedes: none
- Change: `openspec/changes/add-endpoint-request-defaults`

## Context

Some models need request settings that most clients never send. On the operator's own Ollama host, `qwen3.5:35b-a3b`
thinks by default and ran a trivial prompt to the 8,000-token limit with an empty answer; with `reasoning_effort: "none"`
it answered in 39 tokens, and with its instruct sampling a fixed task set went from 79 to 85 of 87 passed. The gateway is
the one place every client's request passes, and the operator is the one who knows the model, so the operator should be
able to say it once. Two things are easy to get wrong: letting that setting replace what a client explicitly asked for, and
putting it somewhere that follows the request to a different model.

## Decision

A request setting the operator wants for a model is declared on **the endpoint** (a provider and a model), is **a default
that fills a field the request does not carry and never replaces one it does**, and is limited to **a short allow-list of
typed, ranged fields** that do not change cost estimation or routing.

- *Scope is the endpoint.* A provider serves many models and a route can fall through to another, so a setting that
  belongs to one model cannot sit on either. The endpoint already carries the other per-model facts.
- *Fill, never force.* The client keeps what it sent. A way to force a value is a separate, named setting if it is ever
  needed, with its own decision, because it hides what a client asked for.
- *An allow-list.* An open map turns a typo into a field a provider rejects, found only when traffic fails. `max_tokens`
  and similar fields that change what a response may cost stay out.
- *Applied to the translated request.* The same defaults reach an endpoint whichever protocol the client spoke.

Alternatives: provider-level or route-level defaults (wrong scope), key-level (the client does not know the model),
forcing by default (surprising), an open field map (typos reach production), and recording a default on every decision
(the HTTP client library merges internally and does not report which fields it added).

## Consequences

Easier: a model's correct settings are written once in the reviewed configuration and every client gets them; the
effective configuration shows them; a client that wants something else still gets it.

Harder, and to keep true: a default can hide a model's own default, so it is shown, not silent. The Anthropic `thinking`
object is not carried to OpenAI-compatible endpoints by the translation library, so a client's wish for thinking can only
be expressed through an effort field; this is documented, not worked around here. The allow-list belongs to a protocol: a native Anthropic upstream
(roadmap) needs its own, since an Anthropic body has no effort field and does not take the other sampling fields. Later
settings (a force, a route or key default) must be added beside this one and must not weaken its rule that a client's own value wins unless a later ADR
says otherwise. Like actors (ADR 0037), a default can only shape a request within what the configuration already allows.
