# Proposal

## Why

`add-baseline-cost` lets an admin ask what the traffic would have cost at an endpoint they choose, but only through
`curl`. The number is meant to be seen: it is the figure that tells someone whether a router is worth running. The query
view is where an operator already looks at the traffic, so it should show this beside it.

## What Changes

- The query bar gains a **Compare with** choice: none (the start) or one of the configuration's priced, untiered
  endpoints, offered from the effective configuration. The choice is kept in the URL fragment as `baseline` and read only
  when it is exactly one of the offered endpoints.
- When one is chosen, a **strip** above the chart says, in words, what this traffic would have cost at that endpoint, what
  it cost, and the difference, with the endpoint's prices, the calls left out and the statement that it is an estimate.
  A difference below zero says "more", not a negative saving.
- Read only: nothing is written, and no new server work. An older gateway that does not know `baseline` shows a notice
  and the view works as before.
- **Out of this change:** a default baseline in the configuration, comparing with more than one endpoint, a column in the
  table, and any other screen.

## Capabilities

### New Capabilities

None.

### Modified Capabilities
- `admin-gui`: the query bar can compare the traffic with a baseline endpoint, and the view says what it would have cost.

## Impact

- **Code:** `ui/src` only: the typed client (a configuration read, and `baseline` on statistics), the query model, the
  query bar, a strip component, their tests and `docs/admin-gui.md`.
- **API, server, ledger, request path:** none; it uses `GET /admin/v1/config` and `baseline` on `GET /admin/v1/stats`,
  both already shipped.
- **Security:** the new fragment value is read against a list built from the gateway's own configuration; any other value
  is none. Endpoint names and the statement come from the API and are shown as text.
