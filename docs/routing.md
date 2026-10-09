# Routing, failover and policy

How a request becomes a provider call. The flow is: decode the client protocol, pick a route,
let the routing policy narrow the eligible targets, run the route's Switchyard algorithm, call
the selected target (failing over across its endpoints), then encode the answer for the client.

## Providers, targets, routes

- A **provider** is an OpenAI-compatible endpoint with an API key from an environment variable
  and optional static `headers` sent on every call (some providers require identifying headers).
  Per-conversation header values, such as a session id, are not supported yet.
- A **target** is a named model served by an ordered list of `{ provider, model }` endpoints.
- A **route** is what clients request as `model`. It wraps a built-in Switchyard algorithm over
  targets. A bare target name also works: it behaves as a passthrough route.

| Route type | Behavior |
|---|---|
| `passthrough` | First target; the rest are fallbacks |
| `random` | Weighted pick (`weights`, `seed`); the rest are fallbacks |
| `stage_router` | Tool-result signals choose `efficient` or `capable` (`mode`, `confidence_threshold`) |
| `llm_classifier` | A `judge` target decides: `capability` mode judges task difficulty, `escalation` mode latches the session to `capable` after `confirmations` verdicts |

Algorithms keep per-session state (escalation latches, capable-hold turns). Clients identify a
session with the `x-switchyard-session-id` header; without it, state-dependent behavior is limited.

## Selecting the route

By default the client's `model` names the route. Selector rules let the operator decide the route
from facts about the request instead, so different clients, keys or agents follow different flows
(for example private, then paid, then public for one client and paid only for CI). Rules are
evaluated top to bottom and the first one that matches wins; with no match the requested model is
the route.

```toml
[[select]]
name = "subagents"                       # optional; shown in x-humpyard-rule
when = { subagent = true }
route = "cheap-first"

[[select]]
when = { profile = "deep" }              # the client sent x-humpyard-profile: deep
route = "paid-only"

[[select]]
when = { tag = { team = "infra" } }      # x-humpyard-tag-team: infra
route = "paid-only"

[[select]]
when = { key = "ci-*" }                  # the authenticated key's id
route = "paid-only"

[[select]]
when = { header = { "x-client" = "openclaw" } }
route = "cheap-first"

[[select]]
route = "default"                        # no conditions: matches everything
```

All conditions of a rule must hold. Text conditions are globs (`*` matches any text, everything
else is literal and case-sensitive). The conditions are:

| Condition | Matches |
|---|---|
| `model` | the model the client asked for |
| `key` | the authenticated key's id (never matches on an open gateway) |
| `profile` | the client's `x-humpyard-profile` header |
| `tag` | `{ name = glob }` against `x-humpyard-tag-<name>` headers |
| `header` | `{ name = glob }` against any other request header (names are case-insensitive) |
| `agent`, `task` | the agent and task ids the client reports (`x-switchyard-agent-id`, `x-switchyard-task-id`) |
| `subagent` | whether the client marked the request as coming from a sub-agent |
| `stream` | whether the client asked for a streamed response |

**Standard client headers.** Clients such as agent harnesses can describe themselves with
`x-humpyard-profile: <name>` and `x-humpyard-tag-<name>: <value>`; they mean nothing until a rule
uses them.

**Clients can narrow, never widen.** A rule only applies if the key may use its route
(`allowed_routes`); otherwise it is skipped and the next rule, or the requested model, applies, so
a header can neither reach a route the key may not use nor turn a working request into a refusal.
Give any key whose clients you do not fully trust an `allowed_routes` list. Credential headers
(`authorization`, `x-api-key`, `cookie`, `proxy-authorization`) cannot be matched. A rule can also
rescue a model name the gateway does not know, which helps clients that hard-code one.

**Explaining a decision.** `POST /v1/route/explain` (with your key) says what a request would do,
without calling a provider or spending anything. Describe the request; only `model` is required:

```sh
curl localhost:8080/v1/route/explain -H "Authorization: Bearer $KEY" -H 'content-type: application/json' \
  -d '{"model":"agent","headers":{"x-humpyard-profile":"deep"},"subagent":false,"stream":false}'
```

The answer has `selected` (`route`, `rule`, and whether it came from a `selector` or the
`requested_model`), `rules` (every rule in order: `matched`, the `mismatches` that stopped it, and
`key_may_use_route`), the key's `budget` state, the `targets` the route would use after the routing
policy with each endpoint's health, the route's `fallback_on`, and an `outcome`: `ok`, `forbidden`,
`unknown_model`, `budget_exhausted` or `no_eligible_target`. It runs the same code as real
requests, so the two agree; it only answers for the calling key.

Responses carry `x-humpyard-route` (the route that served) and `x-humpyard-rule` (the rule's name,
`select[<index>]` when unnamed, or `default` when the requested model named the route); the log
line has both and the ledger's route column records the route. Selection happens before the budget
check and the routing policy, which then apply to the chosen route. Loading the config fails when
a rule names an unknown route, an unknown condition, a credential header, repeats a name, or
follows a catch-all (it could never match). See [ADR 0011](adr/0011-selectors-choose-the-route.md).

## Failover

- **Within a target**, endpoints are tried in order. The gateway moves to the next endpoint on
  connection errors, timeouts, HTTP 408, 429 and 5xx. Other 4xx responses stop immediately
  (another endpoint would reject the same request). Nothing fails over once a stream has started.
  `max_retries` on a provider sets extra attempts on that provider *before* failing over.
- **Skipping failing endpoints.** Failing over on every request would make a dead provider cost a
  full timeout each time, so each endpoint (provider + model) has a circuit breaker; see
  [Endpoint health](#endpoint-health).
- **Across targets**, if every endpoint of the selected target fails, the next target the
  algorithm returned is tried. By default falling through happens for a context-window overflow,
  429, 403, 408, any 5xx, connection errors and timeouts. Other client errors (400, 401, 404) stop
  at once and reach the client, because another target would reject the same request. If no
  target answers, the client gets the last error (a timeout is still reported as 504).
- **Choosing what falls through.** A route can limit this with `fallback_on`, a list of failure
  classes (`overflow`, `rate_limit`, `timeout`, `server_error`, `connection`, `forbidden`). A
  failure outside the list ends the request and the client sees the original status and message.
  `fallback_on = []` turns target fallback off. This keeps a paid last resort from being spent on
  a public blip:

  ```toml
  [routes.deep-public]
  type = "passthrough"
  targets = ["pub-deep", "paid-deep"]
  fallback_on = ["overflow"]     # go to paid only when the prompt does not fit
  ```

  It applies to every route type and governs the hand-over between targets; endpoints inside one
  target are a rotation and always fail over.
- If everything fails, the client gets the last error: 429/4xx pass through, unreachable is 502,
  timeout is 504.

Responses carry `x-humpyard-target` (who served) and `x-humpyard-provider` (which endpoint).

## Endpoint health

After `failure_threshold` consecutive failures of the kind that trigger failover (connection
errors, timeouts, HTTP 408, 429, 5xx) an endpoint is skipped for `cooldown_secs`. Each time it
trips again the cooldown doubles, up to `max_cooldown_secs`. When a cooldown ends, exactly one
request is sent as a probe: success returns the endpoint to service at once and resets the
cooldown; failure reopens it for longer. Other 4xx responses say nothing about the endpoint and are
never counted.

```toml
[health]               # all optional; these are the defaults
failure_threshold = 3  # 0 turns health tracking off
cooldown_secs = 30
max_cooldown_secs = 300
```

- **Health never denies service.** If every endpoint of a target is cooling down, the target is
  still tried, soonest-to-reopen first.
- State is in memory per process: a restart starts everything healthy. Failures after a stream has
  begun are not counted (nothing fails over then either).
- The log shows each transition ("endpoint is failing; skipping it for the cooldown",
  "endpoint recovered"), and `GET /v1/health` (with a key when the gateway has keys) lists every
  endpoint's `state` (`healthy`, `cooling_down`, `probing`), `consecutive_failures` and
  `cooldown_remaining_ms`.
- Not yet: health does not make a whole target ineligible for tier substitution (a policy cannot
  know a route has a healthy alternative, so that needs route topology and a fall-back step), and
  `Retry-After` is not honored. See [ADR 0009](adr/0009-endpoint-health-lives-in-the-pool.md).

## Routing policy

A policy decides, per request, which targets are eligible. Ineligible targets are removed from
every group the algorithm sees, including its fallbacks.

- **Tier substitution:** if the policy removes every `capable` target, `capable` is served by the
  eligible `efficient` targets (and the reverse; a missing `judge` falls back to a remaining tier).
  Limits therefore degrade routing instead of failing requests.
- **Random routes** re-align their weights to the remaining targets.
- If **nothing** is eligible, the request fails with HTTP 503 naming the route.

The gateway always applies the budget policy (see [budgets.md](budgets.md)); an embedder-supplied
policy (default `AllowAll`) is composed with it, and a target must satisfy both. Embedders implement
`humpyard::policy::RoutingPolicy` and start the server with
`server::router_with_policy`. A policy receives the route, the session id and request metadata.
Health-aware target eligibility is a planned change built on this seam.
