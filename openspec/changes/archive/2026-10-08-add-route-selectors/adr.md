# ADR Review

## Reviewed

ADR-0008 (routing policy seam): policy answers *which targets may serve this caller* (budget,
later health); selectors answer *which route this request follows*. They compose: selection first,
then eligibility on the chosen route. ADR-0003 (config-first keys behind KeyStore): selectors
reference key ids and rely on `allowed_routes`; managed keys will keep working with them.

## Outcome

A durable decision about how routing is made customer-definable, and its safety rule.

## New ADRs

- 0011 Selectors choose the route from request facts; clients narrow, never widen.
