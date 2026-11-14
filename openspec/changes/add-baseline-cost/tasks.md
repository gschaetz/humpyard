# Tasks

## 1. The sums the baseline needs

- [x] 1.1 Add the count of answered calls with known usage and the group's total cost to the usage aggregate of `ledger/stats.rs` and to `GroupStats`; verify with ledger tests that a group with some calls missing usage reports the right count and cost, and that the existing figures (average cost, tokens, signals) are unchanged

## 2. Pricing a baseline

- [x] 2.1 Add the linear cost of a set of token sums at a price to `pricing.rs`, next to `cost_micro_usd` and by the same rules (cached price or input price, reasoning inside output), and the lookup of a baseline endpoint's price among the configured targets with the three refusals (unknown, no price, tiered); verify with unit tests, including that pricing one call with the new function equals `cost_micro_usd`

## 3. The parameter and the answer

- [x] 3.1 Accept `baseline` on `GET /admin/v1/stats` and refuse it on `GET /v1/stats`; add the per-group baseline cost, actual cost, saving, ratio and priced calls, the top-level `baseline`, `summary` and statement, and the count of calls left out; verify with integration tests for free traffic against a paid baseline, partly paid traffic, a group with no priced calls, every grouping adding up to the summary, calls with no usage, the three refusals, the client refusal, a hostile value, two baselines not sharing a cached result, and an answer without a baseline equal to what it was
- [x] 3.2 Describe the parameter and the new fields in `docs/openapi/admin-v1.json`, regenerate `ui/src/api/schema.d.ts`, and update `docs/api.md`; verify the OpenAPI contract test passes against a real baseline answer and fails when a documented field is removed

## 4. Documentation

- [ ] 4.1 Add the baseline to the statistics section of `docs/budgets.md` and replace the hand comparison in `docs/cut-agent-costs.md` with a real baseline query, with what the figure leaves out stated plainly; verify the documented command runs as written against a gateway and the strict docs build passes

## 5. Wrap-up

- [ ] 5.1 Run the isolation test with the baseline parameter added to the statistics client of its load and record it as the next run in `docs/performance.md`; verify the performance gate passes and the loaded p99 is within the allowed rise
- [ ] 5.2 Check the threat model of `design.md` against what was built (each row true, each proof present), update `docs/threat-model.md` if a gap or boundary changed, update `docs/architecture.md` (the status table and the "Last updated" line) and `docs/roadmap.md` (the savings item moves to shipped), and verify the Mermaid diagrams still match the code
- [ ] 5.3 Run fmt, clippy `-D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items`, the full test suite with and without the `ui` feature, the `ui/` checks and `openspec validate add-baseline-cost --strict`; verify each exits 0

## Workflow follow-up

- Release: the next minor, nothing changes for existing deployments.
- Next, as its own small change: the figure in the GUI's query view (a baseline picker in the query bar and a strip saying what the traffic would have cost, with the basis and the calls left out), read-only. The other read-only GUI screens (target detail, the catalog, keys) are undecided.
- Separate changes: a configured default baseline; tiered baseline prices; the figure in `ctl` (which uses a client key); using it in the launch posts.
