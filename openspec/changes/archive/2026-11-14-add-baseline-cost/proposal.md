# Proposal

## Why

The statistics say what the traffic cost and how it was served, but not what it would have cost without the router. That
is the number that decides whether a gateway is worth running, and today it exists only as a hand comparison in the demo
(two keys, one all-to-one-model, one through `auto`). The operator's own launch notes say it: "no counterfactual 'all on
one big model' cost exists, so no percent-saved claim." This change computes it from what the ledger already records,
priced at a baseline endpoint the operator chooses, and says plainly that it is an estimate.

## What Changes

- `GET /admin/v1/stats` accepts `baseline=<provider>/<model>`, an endpoint named in the configuration that has a price.
  Each group then also reports what its answered calls would have cost at that endpoint's prices, what they actually cost,
  the difference and its ratio, and how many calls were priced. A `summary` adds up the groups, and a `baseline` object
  names the endpoint, its three prices and what the figure is (an estimate from the tokens the calls used, not a
  comparison of answer quality).
- Calls whose usage the provider did not report are left out of both sides and counted, so the two figures always cover
  the same calls.
- A baseline that is not a configured endpoint, has no price or has a tiered price is refused with a message that says
  which. The client statistics (`GET /v1/stats`) do not accept `baseline`: it would show a key the prices of endpoints it
  may not use.
- Documentation: the statistics guide and the cost guide use the real figure in place of the hand comparison.
- **Out of this change:** a default baseline in the configuration, tiered baseline prices, showing it in the GUI or the
  `ctl` command (which uses a client key), and any claim about quality.

## Capabilities

### New Capabilities

None.

### Modified Capabilities
- `routing-statistics`: statistics can price the same calls at a baseline endpoint, and say what the figure is.

## Impact

- **Code:** the usage aggregate in `ledger/stats.rs` gains the count of calls with known usage and the group's total cost
  (both from the query that already runs); the parameter, the arithmetic and the response fields in `server/stats.rs`; a
  small pure function in `pricing.rs`. No new module, no new table, no new row read.
- **Request path:** none. The statistics are a heavy read behind the existing gate and shared result; the change adds two
  sums to a query the read already makes. It carries one isolation run (ADR 0036).
- **API:** additive fields and one optional parameter; the OpenAPI document and the API guide change.
- **Configuration:** nothing is required, and existing deployments are unchanged.
