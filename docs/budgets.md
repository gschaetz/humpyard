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

## Prices and cost

Each endpoint may declare `price = { input, output, cached_input }` in USD per million tokens.
Cost is computed from the tokens the provider reports and kept in whole micro-USD. A config with
any USD budget must price every endpoint (use `0` for free models), so nothing is silently free.
Reasoning tokens are part of the output tokens the provider reports (they are recorded as detail,
not added again), and cache-read tokens bill at `cached_input` (default: `input`).

## The usage ledger

`[ledger] path = "…"` is a SQLite file. Every upstream call is one row: time, key id, session,
route, target, provider, model, kind (`answer`, or `judge` for classifier calls the client never
sees), token counts, cost and outcome (`ok`, `error`, `cancelled`). Judge calls are charged to the
same key. Rows are written by a background task and never delay a response; a full queue or a
failing database is logged and counted, not surfaced to clients. Streams are recorded when they end
(or when the client disconnects, as `cancelled`; a request cancelled before it answers still
records the judge calls it already paid for); a provider that reports no usage is recorded with
zero tokens and `usage_missing = 1`.

On SIGTERM or Ctrl-C the gateway drains in-flight requests, cancels what is left after
`shutdown_grace_secs`, and **flushes the queue to the ledger before exiting**; if it cannot (10
second limit) it exits non-zero and logs that entries may be lost. A hard kill (SIGKILL, a crash,
power loss) can still lose entries that were queued but not yet written, so budgets rebuilt after
one may be slightly low.

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
