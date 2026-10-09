# Coming from a LiteLLM-style front door

For gateways that expose named model groups (say `fast`, `deep`, `coding`), each in public,
private and paid tiers, with fallbacks between tiers and rotation pools of paid models.
[tests/parity.rs](../tests/parity.rs) builds exactly that shape with mock providers and checks
every behavior below, so the table is verified, not aspirational.

## Mapping

| In a LiteLLM config | In humpyard |
|---|---|
| `model_name` group clients request | a **route** (`[routes.<name>]`); a bare target name also works as a route |
| several deployments under one group (a rotation pool) | one **target** with ordered `endpoints`; a 429/5xx/timeout moves to the next, and [endpoint health](routing.md#endpoint-health) skips a model that keeps failing |
| even load across deployments | a `random` route over single-model targets (weights, seed); other targets are its fallbacks |
| `fallbacks` (tier A falls to tier B) | the route's `targets` list, tried in order |
| `context_window_fallbacks` | the same list with `fallback_on = ["overflow"]` on the route, so only an overflow falls through |
| `api_base` + `api_key` | a **provider** (`base_url`, `api_key_env`) |
| `extra_headers` | provider `headers` (sent only to that provider) |
| `request_timeout`, `num_retries` | provider `timeout_secs`, `max_retries` |
| virtual keys, per-key budgets | `[keys.<id>]` (hashed), `daily_usd` etc., `allowed_routes` |
| spend logs | the SQLite ledger (route, target, provider, model, tokens, cost per call) |

## Verified behavior (tests/parity.rs)

- Each tier is served by its own upstream; paid groups are reachable by route.
- A down private tier falls back to public and then paid; a hung one (timeout) falls back too.
- A prompt too large for the public tier goes to paid; a small one stays public.
- With `fallback_on = ["overflow"]`, a public blip (503, 429, timeout) is reported to the client
  and paid is never called; only an overflow reaches paid.
- Client errors (400/401/404) do not fall back, so malformed requests never spend paid credits.
- A paid model over its quota (429) is rotated out after repeated failures, is not hammered while
  cooling down, and rejoins after one successful probe. If every model in a pool is limited, the
  client sees the 429.
- A `random` route spreads load and routes around a rate-limited model without client errors.
- A provider's static header reaches only that provider, each provider gets its own credential
  (never the client's key), requests without a key are refused, and usage is recorded per
  route, target and model.

## Differences to know about

- **Fallback is broader by default.** A tier list falls through on any failover-class error (429,
  5xx, timeout, overflow, connection failure), not only on context overflow. If the second tier is
  paid, a public-tier blip would spend paid credit; set `fallback_on = ["overflow"]` on that route
  to match LiteLLM (verified in the suite).
- **Names are separate.** A route and a target cannot share a name (config validation rejects it),
  so name targets by provider tier (`pub-fast`) and keep client-facing names for routes.
- **Ordered pools are sequential, not balanced.** The first model in a target takes all traffic
  until it fails or hits its quota; use a `random` route for even spreading.
- **Retries are broader.** Provider `max_retries` retries any retryable failure (429, 5xx,
  connection errors) on the same endpoint before falling through; LiteLLM's rate-limit retry
  policy retried only rate limits. With `max_retries = 2` a failing 503 costs three attempts
  (measured: about a second of backoff). Use `max_retries = 0` where the first failure should
  count.
- **Picks are per request, not per conversation.** A `random` route or an ordered pool chooses
  anew for every request, so a multi-turn tool conversation can move between models in the pool
  (as LiteLLM's shuffle did). Nothing is sticky yet.
- **Many SDK clients send no identity.** An agent built on the OpenAI JS SDK sent only a
  generic user-agent: no session id, agent id or metadata. Use one key per consumer with
  `allowed_routes` to tell consumers apart, and, for clients that can add request headers, the
  `x-humpyard-profile` and `x-humpyard-tag-*` headers.
- **`openai/<name>` aliases** (LiteLLM's `model_group_alias`) are one selector rule each:
  `when = { model = "openai/fast" }`, `route = "fast"`.
- **Streams fail over only before the first byte**, as in LiteLLM; a stream that breaks midway
  ends with an error event.
- **No admin UI or database.** Keys and routes are config; see the
  [operator-surface direction](architecture.md#direction-operating-humpyard).
- **Not covered by the suite:** parameter dropping for providers that reject unknown fields,
  tag-based routing, and load balancing by latency or current usage.
