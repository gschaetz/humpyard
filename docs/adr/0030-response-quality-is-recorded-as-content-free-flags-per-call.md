# 0030. Response quality is recorded as content-free flags per call

- Status: proposed
- Date: 2026-10-27
- Supersedes: none
- Change: `openspec/changes/record-response-quality-signals`

## Context

The gateway ranks free models with a `quality` number an operator types, and judges a call by its HTTP outcome. In October 2026 that was not enough: a free model answered 93% of its calls "ok" while ending turns with cut-off tool calls that stuck Codex sessions on strict providers, and a scheduled job failed twice in nine runs on a malformed tool call returned with a 200. A pool cannot learn which of its members answers badly from the status code. The wish is for quality to follow what models actually do, in the end automatically, but the gateway may not keep prompts or responses (ADR 0004, ADR 0013), and a formula nobody has tested must not steer traffic.

## Decision

The gateway records facts about each answer as content-free flags, one set per upstream call, on the call's usage row, attributed to the target, provider and model that produced it. The first four are `empty_output`, `truncated_output`, `bad_tool_call` and `cut_stream`. A flag states only that something was true of the response; it keeps no text, no tool name and no argument, and is observed without changing, delaying or failing the response. A value is unknown (not "false") for a call that was not observed.

The flags are facts, not scores. Turning them into a quality value is a separate step, taken in stages: a computed value shown beside the declared one with no effect on routing, replayed over the stored history, and only then applied to routing, within bounds, with a per-target freeze and one global off switch. Nothing in this decision lets a flag influence which target serves a request.

Alternatives considered: counters only (cannot be replayed against a later formula, and are lost on restart); a model-graded score of the answer's content (needs the content, which the ledger may not hold, and costs a call per answer); storing the flags on the decision (the decision is written before a stream ends, and decisions are pruned while usage rows are not); adjusting quality directly from the first flags (an untested formula steering traffic).

## Consequences

Operators can see per target how often answers are empty, cut off by the length limit, carry a malformed tool call or end early, and a later change can replay any proposed formula over real history before it is trusted. Each new kind of fact is one more column on the usage row. The rule to keep true: a flag never carries content, never changes a response, and never routes by itself until a later decision, with its own bounds and off switch, says so. Facts about a response that only appear later (a bad call found in the next request's history) need their own attribution step and are not covered here.
