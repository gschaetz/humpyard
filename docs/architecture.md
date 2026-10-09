# Architecture

Living document. Update it in the same PR as any change that alters structure, request flow or
component status (see [AGENTS.md](../AGENTS.md)). Diagrams are Mermaid and render on GitHub.

Last updated: 2026-10-08 (`add-route-selectors`: routing rules on request facts; next steps of the same direction are request-feature conditions, an `explain` call and hot reload).

## Component status

| Component | Status | OpenSpec change |
|---|---|---|
| Inbound endpoints (OpenAI Chat, Responses, Anthropic) | Implemented | `add-core-gateway` |
| Protocol translation (Switchyard IR/codecs) | Implemented | `add-core-gateway` |
| Streaming proxy over Switchyard's `run` + `TranslatingLlmClient` | Implemented | `add-switchyard-routing` (group 3) |
| TOML config (providers, targets, routes) + CLI (`serve`, `check-config`) | Implemented | `add-core-gateway`, `add-switchyard-routing` (group 2) |
| Switchyard routing: passthrough, random, stage_router, llm_classifier; per-session state; whole-target fallbacks | Implemented | `add-switchyard-routing` (group 4) |
| Provider pool: multi-provider targets with ordered-endpoint failover | Implemented | `add-switchyard-routing` (group 3) |
| Routing-policy seam (eligibility hook, tier substitution, 503 when none eligible) | Implemented | `add-switchyard-routing` (group 5) |
| Virtual keys (hashed, `KeyStore` trait), usage ledger (SQLite, async), per-endpoint pricing | Implemented | `add-cost-tracking` |
| Budgets: UTC daily/monthly USD+token limits, restricted/exhausted states, 402, free-only, `/v1/key/info` | Implemented | `add-cost-tracking` |
| Route selectors: ordered rules choose the route from the key, headers, tags and agent metadata; clients narrow, never widen | Implemented | `add-route-selectors` (ADR 0011) |
| Endpoint health: per-endpoint circuit breaker with cooldown and probe, fail-open, `GET /v1/health` | Implemented | `add-endpoint-health` (ADR 0009) |
| Health-driven target eligibility in the routing policy (tier substitution on a cold target) | Planned | not yet proposed |
| Engineering hardening: invariants + architecture test, lints, structure refactors, CI gates, property tests, ADRs | Implemented | `harden-engineering` |
| Graceful shutdown: drain, grace period, cancel, ledger flush before exit | Implemented | `add-graceful-shutdown` |
| `count_tokens` / `responses/input_tokens`: local, conservative prompt-size estimates (no upstream call). `responses/compact` intentionally not implemented | Implemented | `add-count-tokens`, `add-responses-input-tokens` |
| Packaging: tag-only release workflow (native runners), GHCR image, launchd service | Implemented (first release not cut yet) | `add-packaging` (ADR 0010) |
| Database-managed keys + a versioned, documented admin API (separate admin credential) | Planned (designed for in `add-cost-tracking`) | not yet proposed |
| `/metrics` endpoint (Prometheus format) | Planned | not yet proposed |
| Read-only status page served by the binary (endpoint health, spend, recent requests) | Planned | not yet proposed |
| Full admin GUI (key and budget management, spend charts) | Later; not ruled out | not yet proposed |
| `migrate-modelrelay` command + bundled catalog | Planned | not yet proposed |

## Direction: operating humpyard

The gateway is configured as code today and observed through `GET /v1/health`, `GET /v1/key/info`
and the SQLite ledger. The planned order for the operator-facing surface is API first:

1. A versioned, documented admin API (OpenAPI generated from the code) with its own admin
   credential and scopes, so a client key can never mint keys. Config-defined keys stay read-only
   in the API; database-managed keys are editable, so config-as-code and runtime management can
   coexist.
2. A `/metrics` endpoint, so existing Prometheus and Grafana setups need no custom UI.
3. A read-only status page served by the binary, built only on the same API.
4. A fuller admin GUI is deliberately left open for later, for people who want one. It should be an
   optional, separate component that is just another client of the admin API, so the core stays a
   single small binary. Nothing above should be built in a way that rules it out.

## Current: request flow (implemented)

```mermaid
flowchart LR
    C1[OpenAI Chat client] --> API
    C2[Codex / Responses client] --> API
    C3[Anthropic client] --> API
    subgraph GW[humpyard]
        API[axum endpoints<br/>decode to Switchyard IR] --> AUTH[Auth<br/>virtual keys, allowlist]
        AUTH --> BUD[Budget check<br/>402 when exhausted]
        BUD --> RT[Routes<br/>built-in algorithm per route]
        RT --> POL[Routing policy<br/>budget + custom]
        POL --> RUN[Switchyard run<br/>selected target + fallbacks]
        RUN --> MET[Metered client<br/>per request]
        MET --> TC[Target client<br/>ordered endpoint failover]
        TC --> EC[TranslatingLlmClient<br/>per endpoint]
        EC -->|IR response or stream| TAP[Stream tap<br/>usage at end]
        TAP --> ENC[Encode to client protocol<br/>SSE framing]
        ENC --> API
        MET -. entries .-> ACC[Accounting]
        TAP -. entries .-> ACC
        ACC --> TRK[Budget tracker<br/>live counters]
        ACC --> LED[(SQLite ledger<br/>async writer)]
        LED -. hydrate on start .-> TRK
        TRK -.-> BUD
        TRK -.-> POL
    end
    EC --> P1[(Provider A)]
    EC --> P2[(Provider B)]
```

## Target: full pipeline (planned order is fixed)

```mermaid
flowchart TD
    IN[Inbound request<br/>OpenAI / Responses / Anthropic] --> DEC[Decode to Switchyard IR]
    DEC --> BUD[1. Auth + budget<br/>implemented]
    BUD --> POL[2. Routing policy<br/>eligible targets<br/>seam implemented, allow-all default]
    POL --> ALG[3. Switchyard algorithm<br/>passthrough / random /<br/>stage_router / llm_classifier<br/>implemented]
    ALG --> POOL[4. Provider pool<br/>endpoint choice + failover<br/>+ circuit breakers<br/>implemented]
    POOL --> UPS[(Providers)]
    UPS --> ENC[Encode to client protocol]
    ENC --> OUT[Response / SSE]
    POOL -. usage events .-> LEDGER[(Async ledger<br/>SQLite, implemented)]
    LEDGER -. refresh .-> BUD
    POOL -. failures / successes .-> HEALTH[Endpoint health<br/>implemented, inside the pool]
    HEALTH -. skip cold endpoints .-> POOL
    BUD -. budget state .-> POL
```

Switchyard decides the macro question (which target); the provider pool answers the micro one
(which endpoint). Policy narrows targets before the algorithm runs; it does not change algorithms.

## Modules (src/)

| Module | Role |
|---|---|
| `config/` | TOML schema (`schema.rs`), cross-field validation (`validate.rs`), env-var key loading and the public `Config` (`mod.rs`) |
| `estimate.rs` | Prompt-size estimate for `count_tokens` (pure function over JSON, bytes-per-token ratio calibrated high) |
| `num.rs` | Checked integer/float conversions for money and token paths (the only lossy casts) |
| `error.rs` | Gateway errors rendered per client protocol |
| `policy.rs` | `RoutingPolicy` trait, `PolicyContext`, allow-all default |
| `auth.rs` | `KeyStore` trait, config-backed store, key hashing, `keygen` support |
| `budget.rs` | Live per-key counters (UTC day/month), budget states, `BudgetPolicy` |
| `ledger.rs` | SQLite usage ledger with async batching writer and period queries |
| `metering.rs` | Per-request metered clients, stream tap, answer/judge classification, `Accounting` |
| `pricing.rs` / `clock.rs` | micro-USD cost from usage; clock and UTC periods |
| `select.rs` | Route selection rules: first-match evaluation of conditions on the key, requested model, headers, tags and agent metadata (pure, no HTTP types) |
| `routing.rs` | Builds one long-lived Switchyard algorithm (and target groups) per route and per bare target; applies policy eligibility, tier substitution and random-weight realignment |
| `pool.rs` | Per-target `RoutedLlmClient`: ordered endpoints, failover, circuit breaking via `health`, provider attribution header |
| `health.rs` | Per-endpoint circuit breaker (cooldown, doubling, single probe) with an injected clock |
| `server.rs` | Router, request stages (decode, authorize, budget, plan, execute, encode), SSE framing, attribution headers; `run` owns the server lifecycle (graceful drain, hard stop, ledger flush) |
| `main.rs` / `lib.rs` | CLI and library root |

Tests: unit tests beside the code; end-to-end suites in `tests/` (shared helpers in
`tests/common/`); `tests/architecture.rs` enforces the layering rules; `tests/switchyard_assumptions.rs`
pins the Switchyard behaviors we rely on; property tests for pricing, budgets and numeric helpers
live in `#[cfg(test)] mod properties` blocks.

## Guardrails

What keeps the structure honest, and where each rule is written down:

- [invariants.md](invariants.md): the rules, each naming its enforcement.
- [adr/](adr/): the durable decisions (immutable).
- CI jobs (all required on `main`): `check`, `deny`, `docs`, `msrv`, `coverage`.
- Lints in `Cargo.toml`: no panics in production code, checked numeric conversions.

## Key decisions

- Reuse Switchyard's IR and codecs instead of hand-written provider types.
- Embed `switchyard-libsy`; the provider pool implements its `RoutedLlmClient` over `TranslatingLlmClient`, and dispatch runs through `switchyard-llm-client::run`.
- Config references API keys by env var only; modelrelay config comes in via a one-shot migration.
- Details and evidence: [background.md](background.md) and `openspec/`.
