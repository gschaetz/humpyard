# Design

Switchyard decides whether to try the next candidate from a fixed list of error kinds (not
configurable). To forbid falling through for a class, the metered client (which knows its
route's setting) re-labels the failure as a general error Switchyard does not fall back on, with
the original status and text encoded in it; `GatewayError::from` restores the original for the
client. This is the same boundary adaptation used for timeouts (change fallback-on-timeout) and
needs no fork of the engine. Classification (`fallback_class`) mirrors the HTTP statuses and error
kinds the pool already treats as endpoint failures. The setting is per route (one value for all
its hand-overs); per-hop conditions will extend the ladder later. If a later Switchyard exposes a
fallback predicate, this adapter can be replaced by it.
