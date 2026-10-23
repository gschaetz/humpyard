# Configuration

See the [example configs](examples.md); `humpyard check-config <file>` validates one. Entries:

- `[providers.<name>]`: an OpenAI-compatible endpoint. The API key is read from the environment
  variable named by `api_key_env`; an inline key is rejected. Optional `headers = { ... }` adds
  static HTTP headers to every call (authentication headers are rejected).
- `[targets.<name>]`: a model served by an ordered list of `{ provider, model }` endpoints.
  Clients may request a target by name; endpoints are tried in order, failing over on
  connection errors, timeouts, HTTP 429 and 5xx (not on other 4xx, and not once a stream has begun).
  Responses carry `x-humpyard-target` and `x-humpyard-provider` headers.
- `[routes.<name>]`: a built-in Switchyard algorithm over targets, requested by clients as the
  `model`: `passthrough`, `random` (weights, seed), `pool = { strength, min_context_window }` in place of `targets` on `passthrough`, `fastest` and `random` (every
  target with that [strength](routing.md#pools-and-strengths), resolved at load), `fastest` (eligible targets ordered by observed speed, with
  `explore`, `min_samples`, `max_age_secs`, `seed`; see [Fastest routes](routing.md#fastest-routes)),
  `stage_router` (tool-result signals pick
  efficient vs capable), or `llm_classifier` (`capability` judges task difficulty, `escalation`
  latches to capable after repeated judge verdicts; both need a `judge` target). Send
  `x-switchyard-session-id` so per-session state (latches, holds) persists across requests.
  If the selected target fails entirely, the next target the algorithm returned is tried; a
  route's `fallback_on = ["overflow", ...]` limits which failures may do that (default: all except `unavailable`, which is on by default only for
  `fastest` and pool routes; see [Routing](routing.md#choosing-what-falls-through)).
- `price_source = "models-dev:<provider id>"` on a provider (optional): where `humpyard prices`
  reads its prices; serving never uses it. See [Keeping prices current](budgets.md#keeping-prices-current).
- `price = { input, output, cached_input, tiers }` on an endpoint (USD per million tokens;
  `tiers` raise the rates above a prompt size): see [Prices and cost](budgets.md#prices-and-cost).
- `quality = 0.74` on a target (optional): how good the model is, 0 to 1, as you state it (a benchmark score
  you trust; never fetched). A pool's `min_quality` keeps only targets that declare enough, and a `fastest`
  route's `quality_weight` blends it with speed; see [Fastest routes](routing.md#fastest-routes).
- `strengths = ["general", "fast"]` on a target (optional): soft labels (what the model is good at) that
  decide which [pool routes](routing.md#pools-and-strengths) include it; they never remove it from a request.
- `class = "private"` on a target (optional): a label (`private`, `public`, `paid`, or your own) that
  decisions and `explain` use to flag a fallback that crosses classes; see
  [Target classes](routing.md#target-classes).
- `allowed_classes = ["private"]` on a route (optional): the route may only use targets of those
  classes; the others are removed before routing, so a private route can never reach a public
  target. Every target the route names must have a class. See
  [Strict routes](routing.md#strict-routes).
- `[[select]]`: ordered rules that choose the route from the key, the requested model, headers
  (`x-humpyard-profile`, `x-humpyard-tag-<name>`, or any named header), agent metadata, and what
  the request contains (estimated prompt size, tools, images). First
  match wins; a rule is skipped when the key may not use its route, so clients can narrow but never
  widen. Responses carry `x-humpyard-route` and `x-humpyard-rule`. See
  [Routing](routing.md#selecting-the-route).
- Auto routing (one `auto` model, pool chosen by intent): a tested recipe in
  [examples/auto-routing.toml](examples.md) and
  [Routing](routing.md#auto-routing-recipe).
- `quota_source = "opencode-go"` on a provider (optional), with `quota_poll_secs` (top level,
  default 60, 1 to 3600) and `quota_stale_secs` (default 300, at least the poll interval): the
  gateway reads that provider's published usage windows while serving; see
  [Provider quota](routing.md#provider-quota).
- `[metrics]` (optional, off by default): the Prometheus `/metrics` endpoint. `enabled`, `listen` (a
  separate address; recommended), `bearer_sha256` (the credential for serving it on the main
  listener), `key_labels` and `key_label_limit`. See [Metrics and alerts](metrics.md).
- `decision_retention_days` (top level, default 90): how long routing decisions and provider quota
  samples are kept; see [Keys, budgets and the ledger](budgets.md#the-decision-log).
- `context_window`, `tools` and `vision` on an endpoint (all optional): what the model can take. A target
  that cannot serve a request (tools, images or an oversized prompt) is removed before routing, only
  when that is certain, and a request no target can serve is refused with 400 `no_capable_target`.
  Undeclared means capable. See [Capabilities](routing.md#capabilities).
- `[health]`: optional endpoint circuit breaker (`failure_threshold`, `cooldown_secs`,
  `max_cooldown_secs`, `max_retry_after_secs`: the cap on a cooldown taken from a retry time the error
  states, default 300) and `skip_cold_targets` (default true: leave a target whose endpoints are all
  cooling down out of a request's plan when another target remains, never refusing a request for it);
  `GET /v1/health` shows which endpoints are being skipped. See [Endpoint health](routing.md#endpoint-health).
- `[overrides]`: optional limits for **runtime overrides**, an operator's expiring instruction to stop using a
  target or provider (OpenSpec change `add-runtime-overrides`). `default_secs` (default 14400, four hours) is
  how long one lasts when its creator names no time, and `max_secs` (default 86400, a day; at most 604800)
  is the longest one may last. Every override expires, and `default_secs` may not exceed `max_secs`. The
  settings take effect through `/v1/overrides`; see [Runtime overrides](routing.md#runtime-overrides).

- `[discovery]` and `discover` on a provider (optional): the settings for **model discovery** (ADR 0022),
  which keeps free pools current from a generated file; see [Keeping pools current](discovery.md), including
  how to schedule it. It is **off until opted in at two levels**: `[discovery] enabled = true` (the master
  switch, default false, needs `file`) and, per provider, `discover = { strengths = ["general"] }` (it also
  needs `price_source`, which names the models.dev provider; `enabled = false` inside it opts that provider
  out again). `[discovery]` also has `file` (the generated file, an absolute path; `humpyard models sync` writes it
  and `serve` reads it), `class` (default `public`, applied to every generated target), `max_added_per_run`
  (default 10), `max_removed_fraction` (default 0.5) and `exclude` (`provider/model` ids never to generate).
  `serve` only **reads** that file, at load and on every reload (the reload poll watches it too), and never
  fetches anything. It is accepted or ignored as a whole: it may contain nothing but `[targets.auto-*]` on
  opted-in providers, every endpoint priced at zero, no model a hand-written target already uses, and never a
  class other than the configured one; otherwise it is ignored with a log line, `humpyard check-config`
  says so, and the gateway serves with the main configuration. Target names starting with `auto-` are
  reserved when `file` is set. `humpyard_discovery_file_state` and `humpyard_discovery_targets` show its state.

Logging is controlled by `RUST_LOG` (default `info`).

**Reloading.** Edit the config file and send SIGHUP (or set `reload_poll_secs` so the gateway
notices by itself, which is how to do it under Kubernetes). The new configuration replaces the old
one atomically; a file that fails to load is rejected and the old one keeps serving, with the
reason in `GET /v1/health`. Running requests finish on the old configuration. `listen`, the ledger
path and `shutdown_grace_secs` need a restart, as does rotating a provider API key. Details in
[Deployment](deployment.md#reloading-the-configuration).

**Shutdown.** SIGTERM or Ctrl-C starts a graceful stop: new connections are refused, requests
already running may finish for up to `shutdown_grace_secs` (default 30; a second signal stops
waiting at once), anything still running is then cancelled (streams end with an error event), and
queued usage entries are written to the ledger before the process exits 0. A kill -9 or crash can
still lose entries that were queued but not yet written. `check-config` validates a file and reports
dangling provider/target references and missing key variables.

Not supported yet: database-managed keys, an admin API,
`previous_response_id` (returns 400).
