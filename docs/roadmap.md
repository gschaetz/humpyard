# Roadmap

What has shipped, what is being built, and what comes next, at a glance. The detail behind each row
lives in the [component status table](architecture.md#component-status), the OpenSpec change named
in it, and the [decision records](adr/README.md). Update this page in the same PR as the change.

Last updated: 2026-10-22. Latest release: 0.12.0. Community: [Discord](https://discord.gg/qfTGr7KTfq) for questions and feedback; GitHub issues for bugs and requests.

## Shipped

| Release | Date | What it added |
|---|---|---|
| 0.1.0 | 2026-10-07 | The gateway: OpenAI Chat, OpenAI Responses and Anthropic Messages in; Switchyard routing; a provider pool with ordered failover; virtual keys, a SQLite ledger and budgets; endpoint circuit breakers; graceful shutdown; local `count_tokens`; release workflow, container image and launchd service |
| 0.2.0 | 2026-10-09 | Hot reload; route selectors (ordered rules on the key, headers, tags, agent metadata); content conditions (prompt size, tools, images); `POST /v1/route/explain`; `fallback_on`; a client parity suite and a reasoning-replay fix |
| 0.3.0 | 2026-10-10 | The decision log: every request's rule, attempts, fallbacks and refusals, with `GET /v1/decisions` |
| 0.4.0 | 2026-10-10 | Routing statistics: `GET /v1/stats` (failure mix, latency percentiles, cost per request) |
| 0.5.0 | 2026-10-12 | Price import and drift check (`humpyard prices import` and `check`, models.dev first); tiered prices; visible cross-class fallback; the docs site; the auto-routing recipe |
| 0.6.0 | 2026-10-13 | `price_source` in config, so one command checks every provider; provider quota: a poller and `GET /v1/quota` (OpenCode Go first) |
| 0.7.0 | 2026-10-14 | Quota history: samples kept in the ledger (schema v3, upgraded in place) and `GET /v1/quota/history`, with the cost of the calls between samples; the roadmap page |
| 0.7.1 | 2026-10-14 | [Try it in a minute](demo.md): a no-key demo (mock provider, real gateway) that routes by request type, explains a decision and compares cost; the humpyard badge, favicon and link previews; `rustls` 0.23.46 (RUSTSEC-2026-0346) |
| 0.8.0 | 2026-10-14 | `/metrics`: Prometheus metrics on one registry (a status page can read the same one later), off by default, on a separate listener or behind its own credential, labels only from configuration, opt-in per-key series; Grafana dashboard, alert rules, ServiceMonitor and NetworkPolicy examples (ADR 0016) |
| 0.8.1 | 2026-10-15 | Strict routes: `allowed_classes` removes targets of other classes before routing, so a private route can never reach a public target (validated at load, recorded on decisions, `humpyard_fallback_blocked_total`, ADR 0017); a summary Grafana dashboard (totals over a time range) and a corrected Switchyard link |
| 0.8.2 | 2026-10-15 | Health-driven eligibility: a target whose endpoints are all cooling down is left out of a request's plan when another target remains, failing open so health still never denies service (`cold_targets` on decisions, `humpyard_health_skips_total`, `[health] skip_cold_targets`, ADR 0018) |
| 0.8.3 | 2026-10-16 | Capability eligibility: endpoints declare `context_window`, `tools` and `vision`; a target that cannot serve a request is removed before routing, only on certainty (a declared limit, or a deliberately low prompt estimate), and a request nothing can serve is refused with 400 `no_capable_target` before any upstream call (`incapable_targets` on decisions, `humpyard_capability_skips_total`, `prices import` proposes the fields, ADR 0019); the README quick start is self-contained |
| 0.8.4 | 2026-10-16 | Session and cache analytics: token totals and `cache_hit_ratio` on every statistics group, `group_by=session`, and `GET /v1/stats/switches` (cached tokens abandoned when a session changes endpoint, priced from the config, with session coverage), `cached_input` and `cache_creation` token directions in `/metrics`; read-only, it measures whether session stickiness is worth building |
| 0.8.5 | 2026-10-16 | `session_id` and `x-session-affinity` are accepted as a session id when no recognized header carries one (so OpenClaw with session affinity on shows up in the session analytics); the example live dashboard aggregates its gauges so rollouts no longer duplicate series |
| 0.9.0 | 2026-10-16 | Free-pool routing, steps 1 to 3 of replacing ModelRelay: a 429 or 503 that states a retry time cools the endpoint for exactly that long (read from the error body until Switchyard exposes the header); a `fastest` route type orders the eligible targets by observed time to answer, with exploration; targets carry `strengths` and `passthrough`, `fastest` and `random` routes can be a `pool` of every target with a strength (and a declared minimum context window), resolved when the configuration loads. Ledger schema v8 |
| 0.9.1 | 2026-10-17 | `humpyard migrate-modelrelay`: prints a config fragment (providers, one target per usable model with its tags as strengths and its context window, and `auto-fastest` / `tag:<strength>` / `+min_ctx:N` pool routes) from ModelRelay's admin API or saved responses; never requests or writes a key and never edits a file; checked against ModelRelay 1.28. See [Moving from ModelRelay](migrating-from-modelrelay.md) |
| 0.9.2 | 2026-10-17 | Free pools that survive and renew themselves: a dead or removed model (404, 401, 403, or a 400 naming a missing model) now counts against its endpoint and, on `fastest` and pool routes, moves the request to the next target (before, a dead first target failed every request of a `fastest` pool); and model discovery, `humpyard models check` and `models sync`, which keep opted-in free providers current from a generated file `serve` reads and never fetches, off until switched on globally and per provider (see [Keeping pools current](discovery.md)). No schema change |
| 0.9.3 | 2026-10-17 | Quality-aware ordering: a target can declare `quality` (0 to 1), a pool can set `min_quality`, and a `fastest` route can set `quality_weight` to blend declared quality with observed speed (default 0, which keeps the pure speed order); `explain` shows qualities and scores; `migrate-modelrelay` declares ModelRelay's measured scores as `quality` and keeps its estimates as comments (`--estimated-quality` to include them). No schema change |
| 0.9.4 | 2026-10-17 | A guarded tail for pool routes: `then = { targets, on }` on a `fastest` pool route names fixed targets (for example a paid one) reached only when a listed failure class (default `overflow`) ends the pool's last member, so a prompt too big for every free model is still served while free models keep failing over among themselves; cold members are retried before the tail is used; `explain` shows the tail. `migrate-modelrelay` gains `--rename FROM=TO` and `--quality-weight W`. [ADR 0024](adr/0024-a-pool-route-can-end-in-a-guarded-tail.md). |
| 0.9.5 | 2026-10-17 | A pool member that answers `410 Gone` (a model removed for good) or `402 Payment Required` (a key with no credit left) is now treated as a dead endpoint, like a 404 or 401: it counts against its health, the request moves on to the next member, and the real status is restored if nobody answers. Found by putting real requests through a live free pool, where one removed model had failed every request. A test pins the exact set of client errors that move a pool on. [ADR 0023](adr/0023-answers-that-describe-the-endpoint-make-it-unavailable.md). |
| 0.10.0 | 2026-10-19 | `humpyard init`: a guided first config. It checks each provider's models live (listed and answering one small request) so a retired model never reaches the file, writes a loopback-only config with `fast` and `deep` targets, proves it with one in-process request, and creates a budgeted key when asked to listen beyond this machine. The README quick start now leads with it; the example configs no longer name a Groq model that was retired |
| 0.11.0 | 2026-10-20 | `humpyard ctl`: a read-only client for a running gateway, local or remote (`status`, `models`, `health`, `stats`, `decisions`, `quota`, `explain`, `whoami`). The key comes only from an environment variable, redirects are never followed, plain HTTP is for loopback only, and exit codes tell a refusal (1) from an unreachable gateway (2). Also: a provider's `413 Request too large` is now `overflow` (ADR 0023, amended): the request moves to the next endpoint, pool member or guarded paid tail without counting against the endpoint's health, and the client gets the real 413 if nothing takes it |
| 0.12.0 | 2026-10-21 | The effective configuration: `GET /v1/config` returns what the gateway is serving, secrets omitted, with each target's and route's source (`file` or `generated`), the pool routes' resolved members and the config fingerprint; a key reads it only with `read_config = true`. The fingerprint is also the metric `humpyard_config_info`, `check-config` prints it, and `humpyard ctl config [--fingerprint]` shows it, so a repository can be compared with a running gateway. Also: `rustls-webpki` and `webpki-root-certs` updated for RUSTSEC-2026-0382 (certificate paths through distrusted emSign intermediates) |

Merged since 0.12.0, not yet released: `humpyard init --out -` prints the configuration to standard output (progress, the one-time key and the test reply go to standard error), so it can be piped, for example into `kubectl create configmap`.

## Building now

| Item | Why now | Spec |
|---|---|---|
| Measure quota against cost | Days of real data decide what a quota percent means, what the status values are and what happens at a cap. No rule should use it before that | `add-quota-routing` 2.1 |

## Next, in order

| # | Item | Depends on | Spec |
|---|---|---|---|
| 1 | Rules that match on quota (`when.quota`), shown in `explain` and recorded on decisions, plus a pacing recipe | the measurement above | `add-quota-routing` step 3 |
| 2 | Expiring runtime overrides: stop sending traffic to a target or provider, or pin a route, for a few hours with no config change and no redeploy (`disable_target`, `drain_provider`, `pin_route`; mandatory expiry, audited, subtract-only; the first `humpyard ctl` command that changes anything). | the effective configuration and `ctl` (shipped) | `add-runtime-overrides` (in progress: store, endpoints and routing built for all three kinds) |
| 3 | Session stickiness: find which clients send a usable session id, and whether a conversation can be pinned to a pool. Soft stickiness (keep a session on its previous target while it stays eligible) only if the switch report (shipped in 0.8.4) shows switching is costly | the switch report (shipped) | `add-auto-routing` 3.1 |
| 3b | Session-level experiments (fast follow): a `random` route that splits per session, not per request, records the arm on the decision and compares cost, latency and errors per arm; quality comparison waits for quality signals | session analytics (shipped) | not yet proposed |
| 3c | Free-pool routing, to replace ModelRelay: `Retry-After`, latency-ordered routes and strengths with pools are shipped in 0.9.0 (the `Retry-After` header itself waits for Switchyard#904); `migrate-modelrelay` shipped in 0.9.1; dead-model handling and model discovery (`models check` and `models sync`) shipped in 0.9.2; quality-aware ordering (declared `quality`, `min_quality`, `quality_weight`) shipped in 0.9.3; what remains is your own migration and then retirement. See [Replacing ModelRelay](#replacing-modelrelay) | capability eligibility and health skipping (shipped) | retire ModelRelay (operator) |
| 3d | Content signals: local, deterministic detectors (private keys and common API-token patterns, `.env`-style secrets, PII-lite such as emails and Luhn-valid card numbers) that produce facts for selector rules (`when.contains`), so a request holding a secret can be routed to a private pool, and with a strict route can never reach a public one. Categories and counts only are recorded, never matches or positions. The default action is to route; `refuse` (a clear 400) is an opt-in per rule. No masking, no response filtering, no third-party calls | nothing (best after strict routes, shipped) | not yet proposed |
| 4 | A demo on real traffic: a Claude Code or Codex session through an `auto` route, with cost and latency from `/v1/stats` (the no-key demo with a mock provider is shipped) | prices in a real deployment | `add-auto-routing` 5.2 |

## Replacing ModelRelay

ModelRelay is a separate Node.js proxy that humpyard can sit in front of today. In a typical setup it backs
the **free pools**: "the fastest free model that has at least N tokens of context" and "the best free
general model". Switching it off needs humpyard to do what those names mean, so the work is staged, and
`migrate-modelrelay` is the last step, not the first:

| # | Step | What it adds |
|---|---|---|
| 1 | Honor `Retry-After` (shipped 0.9.0, from the error body; the header follows Switchyard#904) | A 429 with a retry time cools the endpoint for that long, not a guessed cooldown (health skipping and capability eligibility are already shipped; "minimum context" is a declared `context_window`) |
| 2 | Latency-ordered targets (shipped 0.9.0) | A route orders its targets by recently observed latency (from the stats humpyard already keeps, no background prober), with some exploration so it does not lock onto one target. This is the latency half of the later "cost-aware ordering" |
| 3 | Strengths and pools (shipped 0.9.0) | A target can carry **strengths** such as `coding`, `reasoning`, `general`, `fast`, `agentic` (what ModelRelay calls tags); a pool route takes every target with a strength (resolved when the config loads, so it stays deterministic and explainable). See [Capabilities and strengths](#capabilities-and-strengths) |
| 4 | A free-model catalog | Bundled facts (providers, models, context windows, tags) with an importer like `prices import`, so a pool does not have to be written by hand. Compiled independently from public data, never fetched while serving |
| 5 | `migrate-modelrelay` (shipped 0.9.1; reads ModelRelay's admin API, see [the guide](migrating-from-modelrelay.md)) | One command reads a ModelRelay setup and writes humpyard config; keys become `api_key_env` references and plaintext is never copied |
| 6 | Retire ModelRelay | The operator's step, once a free pool has run on humpyard alone |

### Capabilities and strengths

Both describe what a model can do, in two tiers with different rules:

- **Capabilities** are hard facts an operator or a public source can state: a context window, tool support,
  image support. They are enforced: a target that cannot serve a request is removed from it, but only on
  certainty ([shipped in 0.8.3](routing.md#capabilities)).
- **Strengths** are soft judgments: what a model is good at. They **never remove a target on their own**.
  They only decide which targets a pool contains and, later, how a route prefers among them, so a wrong
  label costs a slightly worse pick, never a refused request.

They are separate fields (capabilities stay on the endpoint, strengths go on the target), and neither is
the request tag a client sends in an `x-humpyard-tag-*` header, which is about the request, not the model.
"Strengths" is the working name (plain "tags" would collide with request tags); it is settled in the spec.

Steps 2 and 3 are the real gap; the rest is plumbing and data. The first user is the free pools of the
reference deployment, so the order follows what removing ModelRelay there needs.

## Planned: the operator surface

In this order, API first. See [Direction: operating humpyard](architecture.md#direction-operating-humpyard).

1. ~~A `/metrics` endpoint (Prometheus)~~ Shipped in 0.8.0; it is built on one registry, so the read-only status page (item 5) is a thin view over it.
2. ~~Health-driven target eligibility~~ Shipped in 0.8.2 (ADR 0018): a cold target is skipped only when an alternative remains.
3. A native Anthropic upstream, for providers that speak Anthropic Messages directly.
4. A versioned admin API with its own credential, exposing the effective configuration with provenance and its fingerprint, and expiring runtime overrides ([ADR 0025](adr/0025-the-ui-has-no-state-of-its-own.md)). Database-managed keys follow, bounded by profiles defined in the file (they can narrow a profile, never exceed it).
5. A read-only status page served by the binary, built on the same API.
6. `migrate-modelrelay`: a command and bundled catalog to bring a ModelRelay setup across. This is the last step of [Replacing ModelRelay](#replacing-modelrelay), which lists what must exist first.

## Later, and only on a measured trigger

- **Quality signals** in the statistics, so cost is never judged without how well a model answered.
  In stages: first free, objective signals (truncated or empty output, refusals, malformed tool
  calls, a retry in the same session); then a sampled, opt-in judge that runs off the request path
  on a target no more public than the request, stores only a score and a reason code (never content),
  and shows counts and spread in `/v1/stats`; calibrated against a stronger judge before it is trusted.
  Requests are grouped by task class (matching rule, tools, size), not by their text.
- **An LLM `classify` route action**, only if the decision log shows a large catch-all share and a costly
  default pool: a classifier call with a timeout and a fallback pool, where a slow, failing or inconclusive
  classifier always falls back. Not built until the numbers ask for it.
- **More price sources** for `prices import` and `check`: the OpenRouter models API and a hand-maintained
  table, beside models.dev.
- **Quota beyond one provider**: per-model quota where a provider publishes it, adapters for other providers,
  and anything that acts on quota without an operator rule (advisory ordering), only after weeks of statistics.
- **Metrics exporters** (OTLP) if asked for, beside the Prometheus endpoint.
- **Cost-aware ordering** inside the set a route allows (its latency half moves up as step 2 of [Replacing ModelRelay](#replacing-modelrelay)), advisory first, after weeks of statistics
  and with quality signals in them; it needs exploration, so a model that scored badly once still
  gets looked at again.
- **An external guardrail hook**, only on demand: one generic rule action that calls a configured HTTP service with a strict timeout and an explicit fail-open or fail-closed choice, so moderation and PII services (Lakera, Presidio, Bedrock and similar) can be used without humpyard embedding each. It puts network on the request path, so it needs its own ADR. Response-side filtering and prompt masking are not planned.
- **Router benchmarking against TwinRouterBench**, which scores routers on coding agents that pick a model
  at every call (task success and real API spend). Its **static track** (970 labelled conversation
  prefixes, scored offline) can be run for free by sending each prefix through the real gateway with mock
  upstreams standing in for the tiers and recording which tier answered; it is next after the session
  analytics. Its **dynamic track** (100 SWE-bench Verified cases through `--base-url`, roughly $25 to $55 a
  run) is on the back burner until the static result says it is worth the spend. RouterBench, RouterArena
  and LLMRouterBench score single-turn question answering, where a router that does not read content has
  nothing to show, so they are not planned.
- **A savings-versus-baseline view** in `/v1/stats`: what the same real traffic would have cost on one
  reference endpoint, priced from the config (the demo does this by hand with two keys). It reuses the
  pricing code of the session analytics and needs its own spec.
- **A classifier-backed rule action**, if the decision log shows the catch-all route is too coarse.
- **Quota per model**, if a provider publishes it; adapters for other providers' usage endpoints.
- **Ledger scale items**, all low priority and each its own spec-first change, started only on the trigger
  named (measurements and method in [Performance and scale](performance.md)):
  - *Retention or archival for the `usage` table*, which is never pruned. Trigger: a ledger past roughly
    a million usage rows (about 2,700 days at the author's rate), or keys shared with other people.
  - *Make `spend_since` seek on the timestamp* (an index or a different query), so a restart is quick on a large
    ledger. Trigger: start-up budget hydration taking more than a second or two.
  - *Let the decisions lookup by request id use its index* (the optional-filter shape defeats it). Trigger: that
    lookup being used on a large ledger, for example from a UI.
  - *Do not block writes while a large prune runs.* Trigger: a prune after a long gap, or a much shorter
    `decision_retention_days`, stalling the writer for noticeable time.
- **A per-request cost ceiling on keys** (`max_request_usd`), designed in
  `openspec/changes/add-request-cost-ceiling` and not built. It guards against one oversized or runaway request
  on a metered provider, so it has nothing to do on a flat-rate plan such as OpenCode Go (whose limit is its quota).
  Trigger: the first pay-as-you-go provider is added, or keys are shared with other people.
- **A fuller admin GUI**, as an optional separate client of the admin API. It produces patches to the config file (a direct write only for a writable file, by opt-in) and never keeps configuration of its own. **One-click apply** (opening a change in the operator's repository through a hook) is deferred until a real need appears.

## Considered, not planned

- **Pinning a session to one target ("sticky sessions").** Measured on the author's gateway on 2026-10-18 with
  `/v1/stats/switches`: over 7 days, 34 mid-session switches across 6 of 11 sessions that carried a session id
  cost an estimated $0.09 of $0.90 total spend (about 10 percent; 18 percent over the last 24 hours, of $0.20). That
  is a large share of a very small number, about five dollars a year at that rate, so it does not pay for the
  complexity. Every switch was between paid OpenCode models on routes that are `random` on purpose, to spread
  load because that plan's quotas are per model; pinning would concentrate a session on one model's quota and work
  against it. Sessions that stayed on one model all cost nothing, so the free pools were not bouncing a session
  between models either. The sample is small and only 33 to 40 percent of answered calls carried a session id, so
  this is descriptive, not proof. Revisit if paid, cache-heavy traffic grows enough that `extra_share` is a
  meaningful number of dollars, or if sessions on a free `fastest` pool are seen switching models often (the
  `by_pair` list in the same endpoint shows it). The method is in [Keys, usage and budgets](budgets.md).

Nothing here is a promise of a date. Items move up when something real needs them, and move out
when the measurements say they are not needed.
