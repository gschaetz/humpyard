# ADR Review

## Reviewed

ADR-0002 (dispatch on Switchyard's client) and ADR-0008 (policy seam) are unaffected: the
adapter sits at our metered-client boundary, and eligibility (policy) is a separate concern from
failure handling.

## Outcome

The customer-defined routing direction (ladders, selectors, conditions) will need an ADR when the
selector model is proposed; this step is a small, self-contained part of it.

## New ADRs

None.
