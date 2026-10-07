# 0001. Embed Switchyard in-process

- Status: accepted
- Date: 2026-10-07 (recorded retroactively; decided 2026-10-06)
- Supersedes: none
- Change: [add-core-gateway](../../openspec/changes/archive/2026-10-06-add-core-gateway), [add-switchyard-routing](../../openspec/changes/archive/2026-10-06-add-switchyard-routing)

## Context

The gateway needs context-aware routing (escalate on tool failures, judge task difficulty). NVIDIA
NeMo Switchyard provides those algorithms. Its crates are published (0.3.0, Apache-2.0):
`switchyard-libsy` (algorithms; makes no network calls itself), `-protocol` (neutral request and
response types), `-translation` (OpenAI Chat, Responses and Anthropic Messages codecs) and
`-llm-client` (an HTTP client and the `run` driver). The earlier idea of bridging a Rust library
into a Node router no longer applies: this gateway is Rust.

## Decision

Depend on `switchyard-libsy`, `-protocol`, `-translation` and `-llm-client`, pinned to 0.3.0, and
run the algorithms in our process. We do not run `switchyard-server` as a separate proxy, do not
use `switchyard-runner` (it builds its own HTTP clients and would bypass our provider pool), and
do not write our own routing algorithms.

Alternatives considered: a sidecar `switchyard-server` in front of our pool (an extra hop and
process, and no way to feed it budget or health information); our own routers (duplicates a
maintained, tested library).

## Consequences

- Switchyard is pre-1.0, so its API can change. Its types are confined to a few modules
  (invariant 1 in [invariants.md](../invariants.md), enforced by `tests/architecture.rs`), versions
  are pinned, and `tests/switchyard_assumptions.rs` pins the behaviors we rely on.
- The published 0.3.0 crates differ from Switchyard's main branch (for example no failure
  cooldown); read the published source, not main, before relying on a feature.
- We inherit Switchyard's behavior where it surprises us (all of a route's targets act as
  fallbacks, so a judge target can end up answering).
- Built-in algorithms are enough for now; custom algorithms stay possible because `Algorithm` is
  a public trait.
