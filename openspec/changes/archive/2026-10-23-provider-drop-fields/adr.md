# ADR Review

## Reviewed

ADR 0021 and 0017 (classes and pools are never rewritten) and the earlier request adaptation for chat providers
(`adapt-requests-for-chat-providers`): the request is shaped for what a provider accepts, never rerouted.

## Outcome

No new ADR: `drop_fields` is a per-provider configuration setting that applies an existing adaptation step
per endpoint. A failover policy for provider-specific 400s would qualify and is deliberately left to a later change.
