# ADR Review

## Reviewed

ADR 0019 (capabilities: remove a target only when the request is certain not to fit) is the model for the rule.
ADR 0017 and 0021 (classes and pools never rewritten) and the budget design (overshoot bounded, never zero) are
consistent: the ceiling subtracts targets and does not alter the request. ADR 0025 plans profile-level ceilings.

## Outcome

No new ADR: this applies an accepted principle to a new input. The "certain to exceed" rule is stated in the spec
and the design.
