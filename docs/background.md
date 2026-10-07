# Background & design intent

Seed notes from the initial design discussion (2026-10-05). Authoritative requirements
live in `openspec/`; this is context only.

## Goal
One Rust binary uniting:
1. **Cost/budget tracking** (LiteLLM-like): virtual keys, daily/monthly caps, token + $ ledger.
2. **Intelligence layer**: NVIDIA NeMo Switchyard (`switchyard-libsy`) for escalation/cascade/stage
   routing, consuming budget state and live provider telemetry as decision inputs.
3. **Provider gateway** (original implementation, modelrelay-style concepts): provider pool, health pings, tag routing
   (`tag:fast`, `tag:reasoning`, `min_ctx:`), load-balancing, failover, request translation, SSE.

## Pipeline (fixed order)
```
inbound (OpenAI/Anthropic)
  -> 1. budget middleware (moka cache + SQLite): reject on hard cap, emit budget tier
  -> 2. Switchyard: history/tool-error/budget/telemetry -> target tier/tag
  -> 3. provider dispatch: pick healthy endpoint for tag, adapt payload, stream SSE, failover
  -> usage events via mpsc -> async SQLite writer -> refresh budget cache
```

## Responsibility split
- Switchyard = macro routing (which tier). Provider layer = micro routing (which endpoint).
- Client owns conversation state; router inspects the messages array (tool errors, turn count),
  with optional session cache (moka, TTL) keyed by `X-Agent-Session` / `X-Switchyard-Stage`.

## Shared state
`Arc<RwLock<ProviderPool>>` of `ProviderMetrics` (cost/M tokens, TTFT avg, rate-limit headroom,
health) written by a background pinger, read by Switchyard each request.

## Planned crates
axum, reqwest, serde/serde_json, clap, tokio, sqlx (SQLite first), moka, tiktoken-rs,
switchyard-libsy (verify actual crate API/availability before relying on the sketches above;
the code in the original discussion was illustrative).

## Phases
1. Core gateway + serde types + SSE streaming proxy to one provider
2. Switchyard coupling (in-process)
3. Provider registry + health pinger + `migrate-modelrelay` one-shot migration (see below)
4. Cost tracking, virtual keys, budgets feeding Switchyard

## Switchyard findings (verified 2026-10-06 against NVIDIA-NeMo/Switchyard v0.3.0)
- **Availability/license:** all crates are on crates.io at 0.3.0 (`switchyard-libsy`,
  `switchyard-protocol`, `switchyard-translation`, `switchyard-llm-client`, `switchyard-runner`,
  `switchyard-server`). Apache-2.0, so compatible with a public repo. Edition 2024, MSRV 1.96.1.
- **Embedding:** libsy makes no network calls. `Algorithm::run_stream(request, Arc<RuntimeModels>)`
  yields `Step::CallModel` items (classifier/judge calls) that the host serves, then `Step::Done`
  with a `RoutingOutcome { selected_model_ids (best first, rest = fallbacks), request, response?, metadata }`.
  `switchyard-llm-client::run` is a ready-made HTTP driver; we can use it or write our own driver
  so judge calls also go through our provider dispatch.
- **Built-in algorithms:** `Passthrough`, `Random`, `LlmTaskClassifier`, `StageRouter`, plus
  escalation, composite, fall-through, plan-execute, advisor-gate, subagent.
- **Custom algorithms:** `Algorithm` is a public async trait (`name`, `route(driver, request)`),
  so budget/telemetry-aware routing can be a custom algorithm wrapping the built-ins.
- **No generic "external signals" input.** There is no budget or telemetry field. Practical
  injection points: (a) build a per-request `RuntimeModels` that only lists targets allowed by
  budget tier and provider health (clean, no fork); (b) wrap built-ins in our own `Algorithm`
  that reorders/filters `selected_model_ids`; (c) post-process `RoutingOutcome`.
- **Targets are bare model ids, not tags.** Switchyard picks a model id; our provider layer must
  map ids (or tag-like categories) to concrete endpoints. Fallback list maps naturally to
  modelrelay-style failover.
- **Session state:** `State` (turn_count, tool_signals, `extra` map) is per session;
  `Metadata.session_id` comes from `x-switchyard-session-id`. Who stores `State` between requests
  (runner vs host) still needs confirming in `switchyard-runner`.
- **Own translation layer:** `switchyard-translation` converts OpenAI Chat, OpenAI Responses and
  Anthropic Messages to/from a neutral IR. Reuse it instead of hand-writing protocol structs
  (this changes phase 1: serde types come from `switchyard-protocol`).
- **Known upstream issue:** buffered upstream work continues after client disconnect (can incur cost).
- **Upstream contribution rules** (if we send PRs): Conventional Commits, DCO sign-off (`-s`).

## modelrelay migration (decided 2026-10-06)
Checked against the fork's `docs/configuration.md` and a live `~/.modelrelay.json`:
- The JSON holds only credentials, provider toggles, custom OpenAI-compatible endpoints, bans,
  tags and pinning. The built-in provider/model catalog lives in code (`sources.js`, `tags.js`,
  `scores.js`) and keys can also come from ~15 env vars, so the file alone is not the whole config.
- The fork keeps changing its own schema (it auto-migrates legacy shapes), and it stores keys in
  plaintext, which conflicts with our env-var-only key rule.
- Decision: no runtime reader. Provide a re-runnable `migrate-modelrelay` command that reads the
  JSON plus env vars and writes our TOML. Keys become `api_key_env` references with a printed list
  of variables to set; plaintext keys are never copied. The built-in catalog is compiled independently as
  bundled data (facts only, no copied files). Custom endpoints, bans, tags and pinning carry over.

## Open questions
- Resolved: session `State` lives inside long-lived algorithm instances keyed by `Metadata.session_id`
  (e.g. `FallThrough` session map); the host only needs to keep instances alive and pass the session id.
- Resolved: embed libsy + llm-client (`run`, `ClientRouter`, `RoutedLlmClient`); our provider pool
  implements `RoutedLlmClient`. `AlgorithmSpec` is public but its builder is crate-private.
- Is the Switchyard server/runner better reused as-is (with our provider layer behind it) or do we
  embed only `libsy` + `translation` and own the driver? Leaning: embed libsy + translation + protocol.
- Postgres support for team-gateway deployments (after SQLite).
