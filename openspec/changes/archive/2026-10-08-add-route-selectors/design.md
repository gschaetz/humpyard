# Design

## Decisions

- **A table, evaluated top to bottom.** First match wins; conditions inside a rule are ANDed.
  Predictable, easy to review in config, easy to explain. An expression language (CEL) is the
  planned escape hatch for what tables cannot say, not the starting point.
- **Selection is part of the authorize stage.** It needs only headers, the key and the decoded
  request, and runs before the budget check and the policy, so budget state and eligibility apply
  to the chosen route's targets exactly as before. The order of checks (invariant 5) is unchanged:
  nothing reaches an upstream before authorization, budget and planning succeed.
- **Skip, don't fail, when the key may not use a selected route.** A header should not be able to
  turn a working request into a 403, nor reach a route the key is not allowed; the rule is
  skipped and the next one (or the requested model) applies. The requested model itself still
  yields 403 when the key may not use it, as today.
- **Standard names plus explicit headers.** `x-humpyard-profile` and `x-humpyard-tag-*` give
  clients a documented, stable contract; any other header can be matched when an operator names
  it. Credential headers are excluded so a rule can neither leak nor key off a secret.
- **A pure core module** (`select.rs`, no Switchyard, no HTTP types) so matching is unit and
  property tested; the server builds the facts (lower-cased headers minus credentials).
- **Transparency**: `x-humpyard-route` and `x-humpyard-rule` on responses, and the ledger records
  the route that served.

## Risks / Trade-offs

- Rules are order-sensitive; validation rejects unreachable rules after a catch-all but cannot
  detect all shadowing. The later `explain` call is the answer to this.
- Matching on arbitrary headers means a client can influence routing within what its key allows;
  that is the point, and `allowed_routes` is the boundary.
