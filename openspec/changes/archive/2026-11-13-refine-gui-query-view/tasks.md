# Tasks

## 1. The time zone choice

- [x] 1.1 Add `zone` (`local` or `utc`, default `local`) to the query model, read against a fixed list from the fragment and written back with the rest of the query; verify with query tests for the default, both values, a hostile value and a round trip through the fragment
- [x] 1.2 Make the chart's time text take the choice: tick labels, the hover text of a point and the axis note naming the zone, with no change to the bucket data; verify with unit tests run with the zone set explicitly (UTC, US Central across a daylight saving change, and a half-hour zone such as Asia/Kolkata) that ticks and hover text are the true local times
- [x] 1.3 Add the `Local`/`UTC` control to the query bar and redraw the chart from the data in memory when it changes; verify with a component test that switching makes no request to the gateway and changes only the text, and that a fragment with `zone=utc` opens in UTC
- [x] 1.4 Update `docs/admin-gui.md` (the choice, the default, the axis note, the half-hour tick case and the fragment value); verify the strict docs build passes

## 2. The ranking order

- [x] 2.1 Order the bars by the chosen metric's value, highest first, groups with no value last and ties by name, and show the call count on a bar whose group had fewer than 20 calls; verify with component tests for requests, p90 latency, a rate with a one-call group and a group with no value, and that the table's order is unchanged
- [x] 2.2 Update the ranking paragraph in `docs/admin-gui.md` (ordered by its metric, thin bars labelled, the table keeps its own order); verify the strict docs build passes

## 3. Wrap-up

- [x] 3.1 Confirm the change touches no request-path code: run `scripts/select-checks.py` on the diff and verify it selects only the UI checks
- [x] 3.2 Look at the rendered view in light and dark (a headless screenshot of the vitest fixtures) with the zone on both values and each metric; verify the ranking order and the axis note read correctly
- [x] 3.3 Run the `ui/` checks (lint, type check, tests, build), `cargo test` with the `ui` feature, fmt, clippy `-D warnings` and `openspec validate refine-gui-query-view --strict`; verify each exits 0

## Workflow follow-up

- Release: the next minor with the other GUI changes, or a patch on its own (nothing changes for the API or existing deployments).
- Separate changes: fading rate points on thin buckets in the line chart; a hover box for the ranking bars.
