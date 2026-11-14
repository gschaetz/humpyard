# Tasks

## 1. The client and the query

- [x] 1.1 Add a configuration read to the typed client and `baseline` to its statistics call (sent encoded, only when set), and a function that lists the offered baselines from a configuration (priced, untiered, first seen wins, dearest output first); verify with client tests that the value is encoded and the call stays under `/admin/v1`, and unit tests for the list including an endpoint with no price and one with tiers
- [x] 1.2 Add `baseline` to the query model, kept in the fragment only when set and read against the offered list; verify with query tests for the default, a round trip, a value that is not offered, and a hostile value

## 2. The choice and the strip

- [x] 2.1 Add the Compare with choice to the query bar and wire it to a statistics read that carries the baseline, reading the configuration at the start and on a manual refresh and hiding the choice when it cannot be read or offers nothing; verify with component tests for choosing, the fragment, a failed configuration read and a gateway that refuses `baseline` (a notice, and the view as before)
- [x] 2.2 Add the strip above the chart: the endpoint and its prices, what the traffic would have cost, what it cost, the difference and share in words ("less", "more"), the calls left out and the estimate statement, as text only; verify with component tests for a saving, a negative one, no priced calls, no strip without a baseline, and markup in the endpoint name and the statement

## 3. Wrap-up

- [x] 3.1 Update `docs/admin-gui.md` (the choice, the strip, the fragment value, what the figure is not) and look at the rendered view in light and dark with each state; verify the strict docs build passes and the strip reads correctly
- [x] 3.2 Check the design's threat model against what was built and run the `ui/` checks (lint, typecheck, tests, build), `cargo test --features ui`, fmt, clippy `-D warnings` and `openspec validate add-gui-baseline-strip --strict`; verify each exits 0

## Workflow follow-up

- Release: with the baseline API as the next minor (0.32.0).
- Separate changes: a default baseline in the configuration; a table column; any other GUI screen (undecided).
