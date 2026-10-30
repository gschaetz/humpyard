# Design

## Context

See proposal.md for why. What the code does today:

- `endpoint_client` (`src/pool.rs`) builds each endpoint's Switchyard backend with `timeout: Some(timeout_secs)`. The library turns that into one absolute deadline, started when the call starts. The deadline covers the retry loop and, for a streamed answer, every chunk after the response has begun: a wrapper (`stream_with_deadline`) races each chunk against the same instant and ends the stream with a timeout error when it passes. That is the message `response did not finish within N ms, retries included` that the log line added in 0.20.0 showed for a stream.
- For a streamed answer the library's call does not return at the response headers: it reads the stream's first decoded event first (so a failure before any output can still be retried) and returns once that event has arrived. So "the call returned" already means "the provider has started to answer".
- A timeout is an `LlmClientError::Timeout`. The pool classifies it as `timeout`, fails over on it, counts it against the endpoint's health, records it on the attempt and, when nothing answers, returns 504. The tap in `metering.rs` ends a stream that yields an error item and records the call as failed.
- In-force ADRs that bound this: 0002 (all upstream calls go through Switchyard's `TranslatingLlmClient`; failover on timeouts, never after a stream has started) and 0009 (endpoint health lives in the pool). Neither says what a timeout covers.
- Observed 2026-10-28 on Claude Code through a deployed gateway (prompts of 62-107k tokens, `timeout_secs = 45` on NVIDIA and others): a stream that began at 34 s was cut at 45 s total, and a non-streamed call timed out twice at 45 s.

## Goals / Non-Goals

**Goals:** a provider that never starts still fails over as fast as before; a stream that is producing is never cut for its length; the failure class, the 504 and the health accounting do not change; no new setting and no migration.

**Non-Goals:** a limit on the total length of a stream; separate numbers for the start and the idle time; limits per route or per request; changing the retry policy; tuning the free pools for large prompts; anything about a buffered answer beyond keeping its limit.

## Decisions

### 1. The gateway owns the meaning of the timeout; the library's deadline is switched off

`endpoint_client` builds the backend with `timeout: None`, so the library applies no deadline of its own, and the pool applies the limits around the call and around the stream. This keeps ADR 0002's dispatch on the library's client and retries (they still run, still with backoff) and changes only who decides when time is up. The same `timeout_secs` field stays the one knob.

*Alternative:* keep the library's single deadline and raise `timeout_secs`. Rejected: the number would have to be both short (so a stalled model fails over quickly) and long (so a long answer completes), which is the conflict that caused the incident.
*Alternative:* patch or fork the library so its stream deadline is relative. Rejected: the library is a dependency we do not carry; the gateway can express the rule from outside with a few lines.

### 2. The wait to start: the call, timed out as a whole

In `call_detailed`, `endpoint.client.call(attempt)` is wrapped in `tokio::time::timeout(timeout_secs, ...)`. Because the library's call returns only after a streamed answer's first event (or a buffered answer in full), this bounds, in one place, the retries and the wait for the provider to begin. When it elapses the future is dropped (cancelling the request) and the pool sees `LlmClientError::Timeout`, built the way the library builds it, so classification, failover, health, attempt records and 504 are unchanged. For a buffered answer the same wrapper is the whole-call limit, which is what the spec asks for.

### 3. The idle limit: a guard on the returned stream

When the response is a stream, the pool wraps it in a guard that waits for the next event with `tokio::time::timeout(timeout_secs, next)`. If it elapses, the guard yields one timeout error item and ends; any event resets the clock. Nothing else about the stream changes (events pass through untouched and in order). The existing tap sees the error item exactly as it saw the library's, so the call is recorded as failed, the client receives the same error event, and the log line (0.20.0) names the reason.

The failure after a stream has begun is not failed over (ADR 0002 is unchanged): the client has already received part of the answer.

### 4. The message says which limit ran out

Three messages, all `LlmClientError::Timeout`: `no response started within N ms, retries included` (a streamed answer that never produced an event), `response did not finish within N ms, retries included` (a buffered call, as today) and `stream was idle for N ms`. The pool tells the first two apart by whether the call was for a streamed request, which it already knows from the request.

### 5. Documentation follows

`docs/configuration.md` and `examples/config.toml` describe the setting in these terms (it has no description today beyond its default of 120). A comment in an operator's config that says the setting "bounds the wait for a response" becomes true as written.

## Risks / Trade-offs

- **A stream that trickles forever.** With no total limit, a provider that keeps sending events slowly (or empty deltas) holds the connection and the client for as long as it does. → The client can always cancel, and the gateway's graceful shutdown ends streams. A total ceiling can be added as its own setting if it ever happens; it is deliberately not part of this change.
- **A model that pauses longer than `timeout_secs` in the middle of a stream** (a long silent reasoning step) is now cut by the idle limit, where before it would also have been cut by the total limit. → Strictly no worse than today. If it shows up, a separate idle setting is the next step.
- **Keep-alive lines.** If the library surfaces a provider's keep-alive comments as events, a hung stream that only sends keep-alives would never be idle. → The tests assert the idle limit against a stream that goes silent; whether keep-alives are surfaced is checked in the first task, and if they are, the guard counts only events that carry output.
- **Retries now live inside the outer limit only.** The library no longer cuts a retry loop on its own deadline; the pool's wrapper does. → The retry count is small and bounded (`max_retries`), and the wrapper bounds their total time exactly as the library's deadline did.
- **Behavior change for anyone who relied on the total limit as a cap on stream length.** → Called out in the proposal and the release notes.

## Migration Plan

1. Ship it in one release. No configuration change, no schema change.
2. Deploy; watch `stream iteration failed` log lines and the `timeout` outcome counts. Roll back by running the previous version.
3. After it has run for a while, a `timeout_secs` of 45 stays as it is: the wait to start is still bounded at 45 s, which is the number chosen for fast failover.

## Open Questions

- Whether an optional total ceiling for a stream (`max_stream_secs`) is wanted as a later setting.
- Whether the idle limit should be its own number once there is data on how long healthy models pause.
