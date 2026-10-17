# Moving from ModelRelay

ModelRelay is a Node.js proxy that benchmarks free models across providers and serves them as virtual
models such as `auto-fastest` and `tag:general+min_ctx:32000`. humpyard can do the same job with its
[`fastest` routes](routing.md#fastest-routes) and [pools](routing.md#pools-and-strengths). The
`migrate-modelrelay` command writes the humpyard configuration for a ModelRelay setup, so the move is a
review and a merge, not a rewrite. Checked against ModelRelay 1.28.

## What the command does

```sh
export MODELRELAY_ADMIN_TOKEN=...            # only if your ModelRelay has an admin token
humpyard migrate-modelrelay \
  --url https://modelrelay.example --token-env MODELRELAY_ADMIN_TOKEN \
  --min-ctx 32000 --out modelrelay-fragment.toml
```

It reads ModelRelay's admin API (`/api/config` and `/api/models`, nothing else) and prints a configuration
**fragment**: a provider for each enabled provider that has a key, a target for each usable model (its tags
become [strengths](routing.md#pools-and-strengths) and its context window a declared `context_window`), and
routes named like ModelRelay's virtual models. It never edits a file, never overwrites `--out`, and never
asks for a key: it writes `api_key_env = "GROQ_API_KEY"` and you set that variable. Before it prints
anything it loads the fragment with the real configuration parser, and it reports what it left out on
stderr.

Offline, from two saved responses (the token is not needed):

```sh
curl -H "Authorization: Bearer $TOKEN" https://modelrelay.example/api/config > config.json
curl -H "Authorization: Bearer $TOKEN" https://modelrelay.example/api/models > models.json
humpyard migrate-modelrelay --config config.json --models models.json
```

`--url` accepts https, or http on the same machine (the token is sent with the request; use a
port-forward otherwise).

## Options for merging into an existing configuration

- `--rename FROM=TO` (repeatable) names a ModelRelay provider differently in the fragment, for a clash with a
  provider you already have: ModelRelay's `opencode` is OpenCode **Zen** (the free tier), so if your config
  already has an `opencode` provider for another plan, use `--rename opencode=opencode-zen`. The new name is used
  for the provider table, its environment variable (`OPENCODE_ZEN_API_KEY`) and its targets' name prefix; a
  rename that repeats another provider's name, or names a provider ModelRelay does not have, is refused.
- `--quality-weight 0.5` writes `quality_weight` on every generated route, so the pools prefer models with a
  declared quality instead of being purely speed-ordered (see [Quality scores](#quality-scores)).

## What you get

| ModelRelay | In the fragment |
|---|---|
| an enabled provider with a key | `[providers.<name>]` with its base URL and `api_key_env = "<NAME>_API_KEY"` |
| a model (not banned, not hidden) | `[targets.<provider>-<model>]`, `class = "public"` (`--class` to change it), `strengths` from its tags (plus `all`), `context_window` |
| `auto-fastest` | `[routes."auto-fastest"]`: a `fastest` pool over every generated target |
| `tag:general` | `[routes."tag:general"]`: a `fastest` pool of the `general` strength |
| `...+min_ctx:32000` | the same with `min_context_window = 32000`, for each `--min-ctx N` you pass |
| an OpenAI-compatible endpoint you added | a provider with its own base URL (named `oai-<id>`) |
| Ollama | a provider with its base URL plus `/v1` |

Clients keep their model strings: the route names are the ModelRelay names, so an existing
`model = "tag:general+min_ctx:32000"` keeps working, as do key `allowed_routes` that name them. A window
variant is written only when some target qualifies (an empty pool is a configuration error), and only
models that **declare** a window count toward `min_ctx`.

## Quality scores

ModelRelay ranks by a blend of speed and a quality score per model. `migrate-modelrelay` carries a score as the
target's `quality` **only when ModelRelay marks it as measured** (its benchmark index); an estimate, which is
ModelRelay's guess from a model's metadata, is written as a comment and not declared, because a declared
quality is a fact that pool floors and rankings rely on. On the reference instance 23 of 53 usable models are
measured. `--estimated-quality` declares the estimates too. To get ModelRelay's behaviour of preferring
capable models, give your `fastest` routes a `quality_weight` (for example 0.5) and, where you want a floor,
a pool `min_quality`; targets with no declared quality rank as neutral and do not qualify for a floor.

## What does not carry over

The report lists each of these, with the reason:

- **Providers humpyard has no OpenAI-compatible base URL for** (for example Kiro) and **disabled providers**.
- **Providers with no key** in ModelRelay, except Ollama and custom endpoints, which are kept (they may
  need none): set the variable to the key if the server wants one, or to any non-empty value if not.
- **Banned, hidden and filtered-out models.** Models ModelRelay's own filters removed (an excluded provider
  or a minimum quality score, status `excluded`) are counted by provider in the report rather than listed;
  bans are named. Models that are merely `down` or `timeout` at the moment are kept: humpyard's health
  tracking handles a failing endpoint.
- **Models with no usable context window** are migrated but declare none, so they are in `auto-fastest` and
  `tag:` routes and absent from `+min_ctx:N` pools, as in ModelRelay. A window ModelRelay only knows as a
  model maximum (not served by any provider) is not declared.
- **Extra keys of a multi-account provider.** humpyard uses one key per provider, so one variable is
  referenced and the report says how many keys ModelRelay has; add the others as extra providers by hand if
  you need them.
- **Tags outside the shared vocabulary** (`coding`, `reasoning`, `general`, `fast`, `agentic`).
- ModelRelay's ping scores, quality ranking, request logs and telemetry. humpyard orders a `fastest` route by
  what it observes itself.

## Dead models do not break a pool

Free providers remove models and revoke access without notice, and a good share of a ModelRelay setup is
`down` at any moment (on the reference instance most of the usable models were). ModelRelay tries the next
model; humpyard does the same on `fastest` routes and pool routes (which is what the migration generates): an
answer of 404 or 401, or a 400 naming a missing model, moves the request to the next target and counts
against the dead endpoint, so after a few such answers it is skipped and probed again later. Other routes you
write keep their old behavior unless they list `unavailable` in `fallback_on`.

## Keeping the pools current: you schedule it

ModelRelay refreshed its model lists by itself every 30 minutes; humpyard does not run anything on its own.
To get the same effect, switch on [discovery](discovery.md) for the free providers and **run `humpyard models
sync` on a schedule** (cron, a systemd timer, launchd, Task Scheduler, a loop in a second container, or a
Kubernetes sidecar; each recipe is in that guide). Without it, a migrated pool is a snapshot: new free models
never join and removed ones linger until health skipping routes around them. A paid plan's new models are
reported by `humpyard models check --paid`, never added automatically.

## Moving over

1. Run the command and review the fragment; merge it into your configuration (or an included file).
2. Set each `api_key_env` variable in the gateway's environment.
3. `humpyard check-config` your merged file, then start the gateway (or reload it).
4. Point a client at one drop-in route first and compare it with ModelRelay using
   [`/v1/stats`](budgets.md#statistics) (cost, failures, latency by target) and `route/explain`.
5. When the free pools have run on humpyard alone, switch ModelRelay off.

The decision to read the admin API rather than `~/.modelrelay.json` is recorded in
[ADR 0007](adr/0007-one-shot-modelrelay-migration.md).
