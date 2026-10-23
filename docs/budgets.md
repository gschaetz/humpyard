# Keys, usage and budgets

Everything here is opt-in. With no `[keys.*]` the gateway is open and unbudgeted, exactly as
before. See [examples/budgets.toml](../examples/budgets.toml) for a complete config.

## Virtual keys

Clients authenticate with gateway-issued keys. Keys are stored **only as SHA-256 hashes** in the
config; the key itself is shown once when you mint it:

```sh
humpyard keygen alice
# Key (shown once; store it securely): sk-humpyard-…
# [keys.alice]
# sha256 = "sha256:…"
```

Paste the printed block into the config. Clients send `Authorization: Bearer <key>` or
`x-api-key: <key>` (Anthropic clients use the latter). Responses to bad keys are 401; a key
restricted by `allowed_routes` that asks for another route gets 403. Logs name the key **id** only,
never the key or its hash. `GET /v1/key/info` shows the calling key's limits, spend and state.

A key can also set `read_config = true` (default `false`) to read `GET /v1/config`, the effective configuration with its fingerprint
(see [`humpyard ctl config`](ctl.md)). Any other key gets 403 `config_not_readable`; the view shows provider
addresses and prices, so give the setting to an operator key and not to a consumer. A gateway with no `[keys]` is open, and so is the view.

A key can also set `manage_overrides = true` (default `false`), which is separate from `read_config`: it is the
permission to create and remove [runtime overrides](routing.md#runtime-overrides), so a key that only watches the
gateway can never switch a target off. Give it to a dedicated operator key, never to a client. A key with `read_config`
can list overrides but not change them.

## Prices and cost

Each endpoint may declare `price = { input, output, cached_input }` in USD per million tokens.
Cost is computed from the tokens the provider reports and kept in whole micro-USD. A config with
any USD budget must price every endpoint (use `0` for free models), so nothing is silently free.
Reasoning tokens are part of the output tokens the provider reports (they are recorded as detail,
not added again), and cache-read tokens bill at `cached_input` (default: `input`).

### Tiered prices

Some providers charge more once a prompt is long. A price can list **tiers**, each with its own
rates, applied when a call's *prompt* (new input, cache reads and cache writes together) is longer
than `above_tokens`:

```toml
price = { input = 3.0, output = 15.0, cached_input = 0.3,
          tiers = [{ above_tokens = 200000, input = 6.0, output = 22.5, cached_input = 0.6 }] }
```

A prompt of exactly `above_tokens` still costs the base price; one token more switches to the tier
(the highest threshold exceeded wins, and a tier's `cached_input` defaults to that tier's `input`).
Thresholds must be above 0 and increasing. For budgets and `restricted_max_output_price` an endpoint
is as dear as its **dearest** tier, and one that is free only up to a size is not free, so a model
that is expensive on long prompts is never mistaken for a cheap one. If a stream ends without
usage, the output-only estimate uses the base rates (the prompt size is unknown), a lower bound.

### Keeping prices current

Typing prices by hand goes stale, so a command can read them from a public source and compare. It
**proposes and reports, and never changes your config or a running gateway**; the gateway itself
never fetches prices (prices come from the config file only).

Name the source on each provider in the config (`price_source` is read only by this command):

```toml
[providers.opencode]
base_url = "https://opencode.ai/zen/go/v1"
api_key_env = "OPENCODE_API_KEY"
price_source = "models-dev:opencode-go"
```

```sh
# Print the price entries the sources propose, for every provider that sets price_source:
humpyard prices import --config config.toml

# Report drift: a price that changed, an endpoint with no price, a model the source no longer lists.
humpyard prices check  --config config.toml
humpyard prices check  --config config.toml --json

# One provider, or a source that is not in the config (needs --provider):
humpyard prices check  --config config.toml --provider opencode --source models-dev:opencode-go
```

`import` also proposes each endpoint's `context_window`, `tools` and `vision` when the source states them
(`limit.context`, `tool_call` and image input), as a line to paste into the endpoint; `check` reports a
declaration the source contradicts as drift, and an undeclared capability as a suggestion only. See
[Capabilities](routing.md#capabilities).

A source is `models-dev:<provider id>`, using the open [models.dev](https://models.dev) dataset (for
example `opencode-go`, `openrouter`, `groq`). Models are matched by exact id; a provider with no
`price_source` is skipped. The JSON report has a top-level `drift` flag and a `reports` list, one per
provider, each with its `provider`, `source` and per-endpoint `status`. `check` exits **0** when everything matches, **1** on drift
and **2** when the source or config could not be used, so a scheduled job can alert on it. Review a
proposal, then copy the entries into your config as you would any other change.

Source data is treated as untrusted: it is fetched over https with a timeout and size limit, every
number must be finite and sane, and a record that is not is reported as unusable rather than
guessed. Sources that publish prompt-size **tiers** have them proposed and compared along with the
base price, so a config missing a tier shows as drift. Peak-hour pricing is not modelled: it is
shown as a note and the base price is kept. On a flat-fee plan (for example
OpenCode Go, which caps each model at a dollar amount of usage priced at these list rates), cost
means *how much of the cap you used*, not money billed.

## The usage ledger

`[ledger] path = "…"` is a SQLite file. Every upstream call is one row: time, key id, session,
route, target, provider, model, kind (`answer`, or `judge` for classifier calls the client never
sees), token counts, cost and outcome (`ok`, `error`, `cancelled`). Judge calls are charged to the
same key. Rows are written by a background task and never delay a response; a full queue or a
failing database is logged and counted, not surfaced to clients. Streams are recorded when they end
(or when the client disconnects, as `cancelled`; a request cancelled before it answers still
records the judge calls it already paid for); a stream that ends without usage (cut by the grace period, dropped client, or a provider that
never reports it) is recorded with `usage_missing = 1` and an **output-only estimate** from the
text delivered (conservative, about 3 bytes per token); its input is not counted. A buffered
response without usage, or a stream that generated nothing, records zero.

On SIGTERM or Ctrl-C the gateway drains in-flight requests, cancels what is left after
`shutdown_grace_secs`, and **flushes the queue to the ledger before exiting**; if it cannot (10
second limit) it exits non-zero and logs that entries may be lost. A hard kill (SIGKILL, a crash,
power loss) can still lose entries that were queued but not yet written, so budgets rebuilt after
one may be slightly low.

## The decision log

Next to the usage rows, the same database holds one **decision** per inference request, so you can
see afterwards what the router did and why, not only what it cost. A decision records: the request
id (also returned as `x-humpyard-request-id`, and stored on the request's usage rows so the two
join), the key, session and protocol, the model the client asked for, the route and selector rule
that applied and whether it came from a `selector` or the `requested_model`, rules that matched but
were skipped because the key may not use their route, the budget state, the targets that were
eligible, the `excluded_targets` a strict route removed, the `cold_targets` skipped for health, the `incapable_targets` removed for capability, the `overrides` (id, kind and subject) that took a target out of its plan, request features (streaming, estimated prompt size when a rule needs it, tools, images),
the **attempts** in order (target, provider, model, class, outcome such as `ok`, `rate_limit`,
`timeout`, `server_error`, and milliseconds), `fallback_crossed_classes` (whether those attempts spanned
more than one [target class](routing.md#target-classes)), the final status and `outcome`, the serving target and
the latency. Refusals are decisions too (`unauthorized`, `forbidden`, `unknown_model`,
`budget_exhausted`, `no_eligible_target`, ...), and a request the client abandons is recorded as
`cancelled`.

It is **metadata only**: no prompts, no responses, error text clipped to 200 characters, and the
requested model name clipped too. It is written by the same asynchronous writer as usage (never
delaying a response, flushed on shutdown), and pruned after `decision_retention_days` (top level,
default 90, `0` keeps everything; changing it needs a restart). An older database is upgraded in
place when the gateway first opens it.

Only decisions (and provider quota samples) are pruned. The `usage` table, the spend history that budgets are
computed from, is kept forever. [Performance and scale](performance.md) lists the retention of each table and how
the ledger behaves at one million rows.

Read it with `GET /v1/decisions` (a key sees only its **own** decisions, newest first; an open
gateway sees all; there is no admin view yet). Filters: `route`, `rule`, `outcome`, `request_id`,
`since_ms` (unix milliseconds) and `limit` (default 50, at most 500):

```sh
curl "localhost:8080/v1/decisions?outcome=forbidden&limit=5" -H "Authorization: Bearer $KEY"
curl "localhost:8080/v1/decisions?request_id=<id from the x-humpyard-request-id header>" -H "Authorization: Bearer $KEY"
```

### Statistics

`GET /v1/stats` summarizes how routes, targets and endpoints have actually performed, computed on
read from the decision log and usage rows (nothing in routing uses these numbers). Parameters:
`window` (`30m`, `24h`, `7d`; default 24h, never longer than `decision_retention_days`),
`group_by` (`route` (default), `target` or `endpoint` as `provider/model`) and the filters `route`
and `target`. Like decisions, a key sees only its own requests.

```sh
curl "localhost:8080/v1/stats?group_by=target&window=24h" -H "Authorization: Bearer $KEY"
```

Per group: `requests`, `attempts`, `outcomes` (attempts by class: `ok`, `rate_limit`, `timeout`,
`server_error`, `connection`, ...), `failures` and `failure_rate`, `fallbacks` and `fallback_rate`
(requests that needed more than one attempt), `latency_ms` (`p50`, `p90`, `p99` over all attempts,
nearest rank, so slow timeouts show in the tail) and, from the usage rows, the `answered` count with
`avg_cost_usd` and `avg_tokens` per answered request, plus the group's `tokens` (`input`, `cached_input`,
`cache_creation`, `output`) and `cache_hit_ratio` (see [Cache use and sessions](#cache-use-and-sessions)).
`truncated` is true when the window held more
than 50,000 decisions and only the newest were summarized. Small samples mislead: look at the counts
before trusting a rate. `POST /v1/route/explain` adds the same recent numbers (last 24 hours: attempts,
failure rate, p50) to each target it lists.

### Cache use and sessions

A model's prompt cache is a cost lever: input tokens served from cache are billed at a fraction of the
fresh price, and a long agent session lives on it. A call's **prompt** is `input + cached_input +
cache_creation` tokens (`input` is the fresh part), and **`cache_hit_ratio`** is `cached_input` over
the prompt, `null` when there are no prompt tokens. Calls whose provider reported no usage are left out,
so they cannot pull the ratio down; a provider that reports no cache usage shows `0.0`, which is what
you were billed.

Two more views answer "what does switching models in the middle of a session cost?". Both read the
usage rows (so they reach back past the decision retention), only for your key, and only for answered
calls that carry a **session id** (the id a client sends, for example in `x-switchyard-session-id`).

```sh
# Your costliest sessions: cost, calls, cache-hit ratio, the endpoints used in order, how many switches
curl "localhost:8080/v1/stats?group_by=session&window=7d&order_by=cost&limit=20" -H "Authorization: Bearer $KEY"

# What switching abandoned, and how many calls carried a session id at all
curl "localhost:8080/v1/stats/switches?window=7d" -H "Authorization: Bearer $KEY"
```

`group_by=session` takes `limit` (1 to 100, default 20) and `order_by` (`cost` (default), `requests`,
`switches` or `recent`); `sessions_total` and `answered_without_session` tell you what the list leaves out.

A **switch** is two consecutive answered calls of one session on different `provider/model` endpoints,
whether a route chose differently or a failover moved the call, because the cache lives with the
endpoint. The switch report gives:

- `coverage`: the share of answered calls with a session id. Look at this first: clients that send none
  give no session view, and everything below covers only the calls that did.
- `switches.warm_tokens_abandoned`: for each switch, what the call before it left cached (its cached
  plus cache-creation tokens), which the new endpoint pays the fresh price for. It is an approximation,
  not an invoice.
- `switches.estimated_extra_cost_usd` and `spend.extra_share`: those tokens priced at the destination's
  configured `input` minus `cached_input` price, as a share of your total spend. Switches to an
  endpoint with no price are counted in `unpriced` and add nothing.
- `by_pair`: the endpoint pairs that switch most.
- `stayed` and `switched`: average calls and cost of sessions that did and did not switch. **Descriptive,
  not causal**: sessions that switch are probably longer and harder.

Use it to decide whether pinning sessions to a target would be worth building: if `extra_share` is a
small fraction of spend, it is not. One measurement is recorded in the [roadmap](roadmap.md#considered-not-planned): it was not worth building.

Not recorded: why a `stage_router` or classifier route picked a tier (the engine does not expose
it; you see the target that served), and attempts that fail after a stream has begun.

## Budgets

Per key: `daily_usd`, `monthly_usd`, `daily_tokens`, `monthly_tokens` over UTC calendar days and
months. Spend counts the instant a call finishes, and is rebuilt from the ledger on restart.

| State | Condition | What happens |
|---|---|---|
| healthy | under `restricted_at` of every limit | normal routing |
| restricted | at or over `restricted_at` of any limit | targets whose output price is above `restricted_max_output_price` are excluded; routing degrades (capable falls back to efficient) |
| exhausted | at or over any limit | `over_budget = "block"`: HTTP 402 before any upstream call. `"free_only"`: only zero-priced targets, 402 if the route has none |

**Overshoot is bounded, not zero.** Cost is known only when a call finishes, so requests already in
flight when a limit is crossed will complete and can take a key past it. New requests are refused
as soon as the limit is observed.

A target's price for these rules is the **highest** price among its endpoints (so a target with an
expensive failover endpoint counts as expensive); a target is free only if every endpoint is free.

## Notes

- Switchyard falls back across *all* of a route's targets when the answer candidates fail, so a
  route's judge target can end up answering as a last resort; that call is recorded as an answer.
- **Moving to database-managed keys later** is designed for: key lookup sits behind a store trait
  and the ledger refers to key ids, never hashes. A managed store adds a keys table, an admin API
  and an import of config keys; the ledger and budgets do not change.
