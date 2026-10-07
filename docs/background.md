# Background & design intent

Seed notes from the initial design discussion (2026-10-05). Authoritative requirements
live in `openspec/`; this is context only.

## Goal
One Rust binary uniting:
1. **Cost/budget tracking** (LiteLLM-like): virtual keys, daily/monthly caps, token + $ ledger.
2. **Intelligence layer**: NVIDIA NeMo Switchyard (`switchyard-libsy`) for escalation/cascade/stage
   routing, consuming budget state and live provider telemetry as decision inputs.
3. **Provider gateway** (modelrelay port): provider pool, health pings, tag routing
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
3. Provider registry port + health pinger + legacy `.modelrelay.json` import
4. Cost tracking, virtual keys, budgets feeding Switchyard

## Open questions
- Verify `switchyard-libsy` public API and license; the examples in the source chat were invented.
- Postgres support for team-gateway deployments (after SQLite).
