# Proposal

## Why

A provider's `timeout_secs` is meant to bound how long the gateway waits for a model, so a stalled free model fails over quickly. In practice it is one deadline, started with the request, that covers the retries and then every chunk of a streamed answer. A healthy stream that is still producing text is cut when the clock runs out. On 2026-10-28 two Claude Code turns on 75-100k-token prompts were cut that way: NVIDIA took 34 s to start and the stream died 11 s later, at exactly 45 s from the request, and a non-streamed call timed out twice at 45 s. The client saw "Server error mid-response" and the turn was lost, even though the model was working. The setting says "bound the wait" and does "bound the whole answer", so raising it to protect long answers also slows every real failover.

## What Changes

- `timeout_secs` bounds the wait for a response to **start** (the request, its retries and the provider's first reply), as before and with the same failover and 504 behavior.
- Once a streamed answer has started, the same number bounds **idle time**: a stream that sends nothing for `timeout_secs` is treated as stalled and ends with the timeout error. A stream that keeps producing is never cut, however long it runs.
- A buffered (non-streamed) answer has no earlier point at which it starts, so `timeout_secs` still bounds the whole call for it.
- The timeout error and its failure class (`timeout`) are unchanged; the message says which limit was hit (the wait to start, the idle time of a stream, or the whole call).
- No configuration change and no migration: the setting keeps its name, default and unit. **Behavior change:** a streamed answer that runs longer than `timeout_secs` in total, with chunks arriving, now completes instead of being cut.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `provider-pool`: what a provider's timeout bounds. It adds the requirements that the timeout limits the wait for a response to start, that a started stream is limited by idle time and not by its total length, and that a buffered answer is limited as a whole.

## Impact

- `src/pool.rs`: the provider's backend is built with no library deadline, and the gateway applies the wait and idle limits itself around the call and around a streamed answer.
- Behavior of `timeout_secs` on every provider; the docs that describe it (`docs/configuration.md` and the example config comments).
- No change to routing, health, the ledger schema, metrics or the API. A failover on a stalled start works exactly as today; what changes is that a long healthy stream is no longer a timeout.
- Not in scope: a limit on the total length of a stream, limits per route or per request, or tuning the free pools for large prompts.
