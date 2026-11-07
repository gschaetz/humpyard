# ADR Review

## Reviewed

ADR-0003 config-first virtual keys behind keystore; ADR-0008 routing policy seam; ADR-0011 selectors
choose the route; ADR-0012 config snapshot reloaded on SIGHUP; ADR-0017 to ADR-0019 (route eligibility);
ADR-0025 the UI has no state of its own. None conflict.

## Outcome

One durable commitment: model aliases are resolved once, at the edge, to a real name, and authorization
and everything after it see only real names.

## New ADRs

- [ADR-0033 Model aliases are resolved once at the edge and never reach authorization](../../../docs/adr/0033-model-aliases-are-resolved-once-at-the-edge.md)
