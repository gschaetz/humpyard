# ADR Review

## Reviewed

ADR 0021 and 0017 (classes and pools are never rewritten) and the `provider-drop-fields` change: another per-provider,
explicit request adaptation applied per attempt.

## Outcome

No new ADR: a per-provider configuration setting in the same pattern as `drop_fields`. Carrying real signatures
would qualify if it needs a new pattern (stateful side channel) and is left for when Switchyard supports it.
