# 0031. A provider's timeout bounds the wait to start and a stream's idle time, not an answer's length

- Status: accepted
- Date: 2026-10-29
- Supersedes: none
- Change: `openspec/changes/bound-the-wait-for-a-response-not-the-whole-stream`

## Context

ADR 0002 sends every upstream call through Switchyard's client, which also supplies the provider timeout. That timeout is one deadline, started with the request, that covers the retries and every chunk of a streamed answer. The setting exists so a stalled free model fails over quickly, but the same deadline cut healthy streams: on 2026-10-28 a Claude Code turn on a 90k-token prompt started streaming at 34 s and was ended at exactly 45 s with `Server error mid-response`, though the model was producing text. Making the number big enough for long answers would make every real failover slow, so the one number could not serve both purposes. The gateway cannot fail over once a stream has begun (ADR 0002), so a wrongly ended stream is a lost turn.

## Decision

The gateway, not the client library, decides what a provider's `timeout_secs` covers, and it covers three different things for three different situations: the **wait for a response to start** (retries included), the **idle time** of a stream that has started (nothing received for that long), and the **whole call** for an answer that is not streamed. A stream that keeps sending is never ended because of its length. The failure class stays `timeout`, so failover, health (ADR 0009), attempt records and the 504 do not change. The library's own deadline is switched off for this; its client, retries and translation are still used as ADR 0002 says.

Alternatives considered: keep the library's single deadline and raise the number (cannot be both short and long); patch the library (a dependency we do not carry); two separate settings now (no data yet on how long healthy models pause; one number keeps the configuration and the migration unchanged, and a second can be added when the data asks for it).

## Consequences

`timeout_secs` now means what its documentation says: it bounds the wait. A long, healthy stream completes. A stream that trickles forever is no longer ended by the gateway, so a total ceiling, if ever wanted, is its own decision with its own setting. The rule to keep true: no limit the gateway applies may end a stream that is still producing, and every limit it applies is reported as a `timeout` that says which one ran out.
