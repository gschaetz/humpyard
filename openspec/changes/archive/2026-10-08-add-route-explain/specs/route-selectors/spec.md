# Spec Delta

## ADDED Requirements

### Requirement: Explain a routing decision without sending the request
The gateway SHALL answer `POST /v1/route/explain`, for the authenticated key, with the selected route and rule, the outcome each rule had (matched, the reasons it did not, and whether the key may use its route), the budget state, the targets and endpoint health the route would use, the route's fallback classes, and an outcome name. It SHALL use the same selection, budget and policy logic as real requests, SHALL NOT call any provider, and SHALL NOT write the ledger.

#### Scenario: Chosen rule and reasons
- **WHEN** a client asks to explain a request with a matching profile header
- **THEN** the report names the chosen rule and route and lists every other rule with why it did not match

#### Scenario: Skipped because of the key
- **WHEN** a rule matches but the key may not use its route
- **THEN** the report shows the rule as matched with `key_may_use_route` false, and the selected route comes from the next applicable rule or the requested model

#### Scenario: Refusals are named
- **WHEN** the request would be refused
- **THEN** the outcome is one of `forbidden`, `unknown_model`, `budget_exhausted` or `no_eligible_target`, with a message

#### Scenario: Agreement with real requests
- **WHEN** the same key, model and headers are sent as a real request
- **THEN** the real response's route and rule equal the explained ones

#### Scenario: Authentication required
- **WHEN** keys are configured and the call has none
- **THEN** it is refused with 401
