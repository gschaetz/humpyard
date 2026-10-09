# Design

Switchyard's `fallback_reason` (published 0.3.0) lists the failures that advance to the next
candidate; `Timeout` is not among them, and the list is not configurable. Rather than fork the
engine, the pool converts a final timeout into `UpstreamHttp { 504, TIMEOUT_MARKER }`, which falls
under the 5xx rule, and `GatewayError::from` converts the marker back to `UpstreamTimeout`.
Endpoint health still records the original timeout (the conversion happens after). Within a
target, endpoints already fail over on timeouts; this change only concerns the hand-over between
targets. If a later Switchyard release falls back on timeouts itself, the conversion becomes a
harmless no-op and can be removed.
