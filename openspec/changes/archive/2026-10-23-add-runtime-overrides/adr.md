# ADR Review

## Reviewed

ADR 0025 sets the rules for runtime overrides: expiry, subtract-only, audited, visible, never widening. This applies them.
ADR 0018 (health fails open) is the one principle this deliberately differs from: an override refuses a request when it
removes the last target, because it is an operator's explicit instruction. ADR 0017 (strict classes) is respected: a pin
cannot cross a class a strict route forbids, and overrides apply after strictness. ADR 0019 (remove only on certainty)
is unaffected.

## Outcome

No new ADR. The fail-closed choice is recorded in the design and flagged for review in the proposal.
