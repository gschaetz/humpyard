# Proposal

## Why

The gateway judges a model by how its HTTP call went: ok, rate limit, timeout. A response can succeed and still be useless, and nothing records it. In October 2026 a free model answered 93% of its calls "ok" while ending turns with cut-off tool calls; each one stuck a Codex session on strict providers until the repair in 0.19.0. `daily-events-scout` failed twice in nine runs with "Provider returned an incomplete or malformed tool call", also from a response that returned 200. A pool's `quality` is therefore a number somebody typed (0.45 for that model, 0.7 for nemotron-ultra), and the pool cannot learn that a model it ranks high keeps ending its answers badly.

The stream recorder already sees every chunk of a response and throws the facts away. Four of them need no content to state: the answer was empty, it was cut off by the length limit, a tool call in it is malformed, and the stream ended before its end. Recording them per call, attributed to the target that served it, is the first stage of letting quality follow what models actually do. Later stages (a computed quality shown beside the declared one, then applied to routing) need history to be tested against, so the facts have to be kept per call, not only counted.

## What Changes

- Every upstream call that returns an answer records four flags, observed while the response passes through and never changing it: `empty_output` (finished normally with no text and no tool call), `truncated_output` (the provider stopped on its length limit), `bad_tool_call` (a tool call with no name, or with non-blank arguments that are not a JSON object), and `cut_stream` (the provider's stream ended or failed before its terminal chunk; a client that disconnects does not count).
- The flags are stored with the call's usage row, which already names the target, provider and model, as four nullable columns (ledger schema v13). A row from before v13, a call that was not observed and a judge call read as unknown, not as zero.
- `GET /v1/decisions`, `GET /admin/v1/decisions` and `humpyard ctl decisions` show the flags that were set on the attempt that served the request, by name, and nothing when none was.
- `GET /v1/stats` and its admin counterpart report, per target, how many calls were observed and how many carried each flag.
- One Prometheus counter, `humpyard_response_signals_total{route,target,signal}`, counts each flag as it is recorded, so a dashboard and an alert can use it without the ledger.
- No content is kept: only whether a flag was set. A tool call's name and arguments are inspected in memory, under a fixed size bound, and dropped.
- ADR 0030 records the commitment: response-quality facts are content-free flags kept per call.

## Capabilities

### New Capabilities

- `response-signals`: what each of the four signals means, when it is set, that it is observed without changing, delaying or failing the response, that its work is bounded, and that it keeps no content.

### Modified Capabilities

- `usage-ledger`: a call's row carries the four flags, unknown for older or unobserved rows.
- `decision-log`: a decision shows the flags set on the attempt that served it.
- `routing-statistics`: statistics report per target how many calls were observed and how many carried each flag.
- `metrics`: each flag recorded is counted per route and target.

## Impact

- Code: `src/metering.rs` (the stream tap and the buffered path), a small pure module that judges a response's facts (registered in `tests/architecture.rs`), `src/ledger.rs` and `src/ledger/stats.rs` (v13 and the per-target counts), `src/ledger/decisions.rs` and `src/server/decisions.rs` (the join and the display), `src/server/stats.rs` and `src/server/admin.rs`, `src/metrics.rs`, `src/ctl.rs`, and the admin API's OpenAPI contract.
- Storage: four nullable integers per usage row. The `usage` table is never pruned, so the flags outlive the decision retention, which is what later stages need to replay.
- API: new optional fields on a decision and on the statistics; existing fields are unchanged.
- Behaviour: none for clients and routing. The flags do not influence which target serves a request in this change.
- Operations: schema v13 cannot be undone, so back up `ledger.db` (with its `-wal` and `-shm`) before upgrading, as for 0.18.0 and 0.19.0.
- Out of scope, and recorded in `design.md` as the next changes: tracing a bad tool call in a later request's history back to the target that wrote it (through the session), a shadow "effective quality" computed from the flags and shown beside the declared one, and applying it to routing within bounds and with a kill switch. Also out: grouping by task class, and judging an answer's content.
