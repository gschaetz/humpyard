# API and endpoints

Clients can speak any of three protocols; providers are reached over OpenAI Chat Completions.
Set `"stream": true` for server-sent events.

```sh
# OpenAI Chat Completions
curl localhost:8080/v1/chat/completions -H 'content-type: application/json' \
  -d '{"model":"fast","messages":[{"role":"user","content":"hi"}]}'

# Anthropic Messages (e.g. Claude Code: ANTHROPIC_BASE_URL=http://localhost:8080)
curl localhost:8080/v1/messages -H 'content-type: application/json' \
  -d '{"model":"fast","max_tokens":100,"messages":[{"role":"user","content":"hi"}]}'

# OpenAI Responses (e.g. Codex CLI)
curl localhost:8080/v1/responses -H 'content-type: application/json' \
  -d '{"model":"fast","input":"hi"}'
```

## Everything else

| Endpoint | What it does |
|---|---|
| `GET /v1/models` | the routes and targets a key may use |
| `GET /healthz` | liveness |
| `GET /v1/key/info` | a key's limits, spend and budget state |
| `GET /v1/health` | which provider endpoints are being skipped, and which config version is loaded |
| `GET /v1/config` | the configuration the gateway is serving, secrets omitted, with where each target and route came from (`file` or `generated`) and the config fingerprint; needs a key with `read_config = true` ([Keys](budgets.md#virtual-keys)) |
| `POST /v1/overrides`, `GET /v1/overrides`, `DELETE /v1/overrides/{id}` | create, list (`?history=true` adds the ones that have ended in the last 30 days) and remove [runtime overrides](routing.md#runtime-overrides): an expiring instruction to stop using a target (`disable_target`) or a provider (`drain_provider`), or to cut a route to one target (`pin_route`). Creating and removing need a key with `manage_overrides = true`; listing needs that or `read_config` |
| `POST /v1/route/explain` | what a request would do, without sending it ([Routing](routing.md#selecting-the-route)) |
| `GET /v1/decisions` | what the router did for your past requests; every response carries an `x-humpyard-request-id` to look one up ([the decision log](budgets.md#the-decision-log)) |
| `GET /v1/stats` | how routes, targets and endpoints have performed: failure mix, fallbacks, latency percentiles, cost per request ([statistics](budgets.md#statistics)) |
| `GET /v1/stats/switches` | what changing endpoint in the middle of a session abandoned: switches, cached tokens left behind, the extra cost at the destination's prices, session coverage ([Cache use and sessions](budgets.md#cache-use-and-sessions)) |
| `GET /v1/quota` | the latest usage windows (percent used, status, reset time) of each provider that names a `quota_source`, with the age of the data, or why it is unknown ([Provider quota](routing.md#provider-quota)) |
| `GET /v1/quota/history` | stored quota samples (`window` such as `7d`, optional `provider` and `name`), oldest first, each with the cost of the provider's calls since the previous sample of its window; needs a ledger ([Provider quota](routing.md#provider-quota)) |
| `GET /metrics` | Prometheus metrics, off by default; on a separate listener or behind its own credential ([Metrics and alerts](metrics.md)) |
| `POST /v1/messages/count_tokens`, `POST /v1/responses/input_tokens` | conservative local estimates: no provider call, no cost |

Verified with real Claude Code, Codex CLI and OpenClaw sessions: see [Clients](clients.md).
