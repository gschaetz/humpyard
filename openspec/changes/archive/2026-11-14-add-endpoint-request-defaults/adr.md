# ADR Review

## Reviewed

All 37 ADRs in `docs/adr/` were listed; none is superseded (no ADR has a `Supersedes:` other than `none`). Read for this
design:

- ADR-0026 A request a pool member rejects moves on within a bound (the `drop_fields` precedent for shaping a request to
  one provider)
- ADR-0034 A configuration patch is typed edits checked by the reload path and written by no one (a refused reload keeps
  the previous configuration serving)
- ADR-0036 Admin and UI work must not change the request path's performance
- ADR-0037 An actor names who a key is for and grants nothing (configuration only narrows or labels, never widens)

## Outcome

One durable decision qualifies: where a per-model request setting lives and what it is allowed to do to a request the
client wrote. It is a contract that later settings (a force, a route or key default, a mapping of the Anthropic `thinking`
object) will build on, and no in-force ADR covers it.

## New ADRs

- [0038 An endpoint default fills what a client leaves out and never replaces what it sent](../../../docs/adr/0038-an-endpoint-default-fills-what-a-client-leaves-out-and-never-replaces-what-it-sent.md)
