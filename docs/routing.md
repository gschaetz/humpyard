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

## Failover

- **Within a target**, endpoints are tried in order. The gateway moves to the next endpoint on
  connection errors, timeouts, HTTP 408, 429 and 5xx. Other 4xx responses stop immediately
  (another endpoint would reject the same request). Nothing fails over once a stream has started.
  `max_retries` on a provider sets extra attempts on that provider *before* failing over.
- **Skipping failing endpoints.** Failing over on every request would make a dead provider cost a
  full timeout each time, so each endpoint (provider + model) has a circuit breaker; see
  [Endpoint health](#endpoint-health).
- **Across targets**, if every endpoint of the selected target fails, the next target the
  algorithm returned is tried.
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
