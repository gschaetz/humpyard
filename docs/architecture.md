# Architecture

Living document. Update it in the same PR as any change that alters structure, request flow or
component status (see [AGENTS.md](../AGENTS.md)). Diagrams are Mermaid and render on GitHub.

Last updated: 2026-10-06 (after `add-core-gateway`; `add-switchyard-routing` is proposed).

## Component status

| Component | Status | OpenSpec change |
|---|---|---|
| Inbound endpoints (OpenAI Chat, Responses, Anthropic) | Implemented | `add-core-gateway` |
| Protocol translation (Switchyard IR/codecs) | Implemented | `add-core-gateway` |
| Single-upstream proxy with SSE streaming | Implemented | `add-core-gateway` |
| TOML config + CLI (`serve`, `check-config`) | Implemented | `add-core-gateway` |
| Switchyard routing (built-in algorithms) | Proposed | `add-switchyard-routing` |
| Provider pool with failover | Proposed | `add-switchyard-routing` |
| Routing-policy seam (eligibility hook) | Proposed | `add-switchyard-routing` |
| Budget / cost tracking, virtual keys | Planned | not yet proposed |
| Provider health + telemetry feeding policy | Planned | not yet proposed |
| `migrate-modelrelay` command + bundled catalog | Planned | not yet proposed |

## Current: request flow (implemented)

```mermaid
flowchart LR
    C1[OpenAI Chat client] --> API
    C2[Codex / Responses client] --> API
    C3[Anthropic client] --> API
    subgraph GW[switchyard-conductor]
        API[axum endpoints<br/>/v1/chat/completions<br/>/v1/responses<br/>/v1/messages]
        TR[Translator<br/>switchyard-translation]
        UP[Upstream client<br/>reqwest + SSE relay]
        API --> TR --> UP
        UP -->|events translated back| API
    end
    UP --> P[(One OpenAI-compatible provider)]
```

## Target: full pipeline (planned order is fixed)

```mermaid
flowchart TD
    IN[Inbound request<br/>OpenAI / Responses / Anthropic] --> DEC[Decode to Switchyard IR]
    DEC --> BUD[1. Budget middleware<br/>planned]
    BUD --> POL[2. Routing policy<br/>eligible targets<br/>proposed seam]
    POL --> ALG[3. Switchyard algorithm<br/>passthrough / random /<br/>stage_router / llm_classifier<br/>proposed]
    ALG --> POOL[4. Provider pool<br/>endpoint choice + failover<br/>proposed]
    POOL --> UPS[(Providers)]
    UPS --> ENC[Encode to client protocol]
    ENC --> OUT[Response / SSE]
    POOL -. usage events .-> LEDGER[(Async ledger<br/>SQLite)]
    LEDGER -. refresh .-> BUD
    HEALTH[Health + telemetry<br/>planned] -.-> POL
    BUD -. budget state .-> POL
```

Switchyard decides the macro question (which target); the provider pool answers the micro one
(which endpoint). Policy narrows targets before the algorithm runs; it does not change algorithms.

## Modules (src/)

| Module | Role |
|---|---|
| `config.rs` | TOML schema, env-var key loading, validation |
| `error.rs` | Gateway errors rendered per client protocol |
| `translate.rs` | Only importer of `switchyard-translation` |
| `server.rs` | Router, handlers, SSE relay |
| `main.rs` / `lib.rs` | CLI and library root |

## Key decisions

- Reuse Switchyard's IR and codecs instead of hand-written provider types.
- Embed `switchyard-libsy`; the provider pool will implement its `RoutedLlmClient`.
- Config references API keys by env var only; modelrelay config comes in via a one-shot migration.
- Details and evidence: [background.md](background.md) and `openspec/`.
