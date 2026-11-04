# ADR Review

## Reviewed

- ADR-0001 Embed Switchyard in-process
- ADR-0002 Dispatch on Switchyard's client; providers speak OpenAI Chat
- ADR-0009 Endpoint health lives in the pool and fails open
- ADR-0018 Cold targets are skipped only when an alternative remains
- ADR-0023 Answers that describe the endpoint make it unavailable
- ADR-0026 A request a pool member rejects moves on, within a bound
- ADR-0028 A decision keeps the provider's scrubbed error text for each failed attempt

All 0001-0029 are accepted and none is superseded; 0030 is proposed (the response signals change) and unrelated.

## Outcome

One durable decision qualifies: what a provider's timeout covers, and that the gateway rather than the client library decides it. ADR 0002 is not changed: dispatch still goes through Switchyard's client, and failover still never happens after a stream has started.

## New ADRs

- [ADR-0031 A provider's timeout bounds the wait to start and a stream's idle time, not an answer's length](../../../../docs/adr/0031-a-providers-timeout-bounds-the-wait-to-start-and-a-streams-idle-time.md)
