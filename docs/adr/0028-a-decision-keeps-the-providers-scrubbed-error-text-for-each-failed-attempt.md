# 0028. A decision keeps the provider's scrubbed error text for each failed attempt

- Status: accepted
- Date: 2026-10-25
- Supersedes: none
- Change: `openspec/changes/record-attempt-errors`

## Context

ADR 0013 says a decision records routing metadata and the ordered attempts and never contains prompts or
responses, and ADR 0004 keeps keys and hashes out of logs and the ledger. In practice a decision keeps one error
text, the last one, cut to 200 characters. With failover (ADR 0026 lets a pool move on from a rejection) a request
can fail on several providers, and all but the last reason is lost. On 2026-10-25 a Codex session was rejected
by three providers in turn and the only record said `rejected` three times, so the cause could not be found
without guessing.

## Decision

A decision keeps, for each failed attempt, **the provider's own error message**, and only that: scrubbed (control
characters and line breaks removed, anything key-shaped replaced) and cut to the same limit as a decision's own
message. It is never the request, never a response body, and never longer than the limit. A decision's own
message goes through the same scrubbing. These messages follow the decision's retention.

Alternatives considered: logging upstream bodies (unbounded, durable only in logs, can echo prompts); a debug
switch that captures full bodies (a prompt store by another name, and a switch someone forgets on); keeping
nothing and relying on replaying a request (needs the request, which we do not keep).

## Consequences

Failures that involve several providers become diagnosable from the decision record alone, by the key that made
the request and by an admin. A provider's error can still echo a fragment of the request; the limit and the
scrubbing bound it, and this ADR is where that residual risk is accepted. The rule to keep true: nothing but a
scrubbed, bounded provider error message is added to a decision, and any new field that would hold request or
response content needs its own ADR.
