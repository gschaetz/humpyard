---
description: How humpyard differs from LiteLLM, OpenRouter and other LLM gateways, and when each is the better choice.
---

# humpyard compared with other LLM gateways

Short version: humpyard is a **self-hosted routing and policy layer for agent traffic** (Claude Code,
Codex and similar). It is not a catalogue of every provider, and it is not a hosted service. If that
is what you need, another tool may fit better, and this page says when.

Other projects change quickly, so the right-hand columns describe what each kind of tool is
generally built for, not a feature checklist. Check their current docs before deciding.

## At a glance

| | humpyard | LiteLLM-style proxy | Hosted router (OpenRouter-style) |
|---|---|---|---|
| Where it runs | your machine, one static Rust binary or a small container | your infrastructure, usually Python plus a database | the vendor's cloud |
| Main job | pick a model per request by what the request is, and explain why | one API in front of many providers | one account and bill for many models |
| Provider coverage | OpenAI-compatible and Anthropic-compatible upstreams | very broad | very broad |
| Client protocols | OpenAI Chat, OpenAI Responses, Anthropic Messages | mainly OpenAI-style | mainly OpenAI-style |
| Routing | selector rules on key, headers, tags, prompt size, tools, images; `stage_router`, `fastest`, `random`, pools | model groups, fallbacks, load balancing; a beta auto router that scores the prompt into complexity tiers | vendor-side model choice |
| Dry run | `POST /v1/route/explain` shows the decision before any call | a test endpoint for the auto router (`/auto_router/test_routing`) | not applicable |
| Keys and budgets | hashed virtual keys in config, daily and monthly caps, per-request ceiling | virtual keys and budgets, usually in a database | account credits |
| Usage record | SQLite ledger, one row per decision with attempts and fallbacks | spend logs, with the auto router's decision and cause in the metadata | dashboard |
| Your prompts leave your network | only to the providers you configure | only to the providers you configure | also through the router |
| Admin UI | not yet ([planned](roadmap.md#planned-the-operator-surface)) | yes | yes |

## Compared with LiteLLM's auto routing

LiteLLM's [auto routing](https://docs.litellm.ai/docs/proxy/auto_routing) is the closest feature to
humpyard's routes, so the differences are worth stating exactly. Checked against its documentation in
November 2026; it is marked beta there, so expect it to move.

- **What is routed on.** LiteLLM's default is a heuristic scorer over the *content* of the prompt (keywords,
  code and reasoning markers), with an optional LLM classifier, sorting requests into `SIMPLE`, `MEDIUM`,
  `COMPLEX` and `REASONING` tiers, each mapped to a model or a pool. humpyard's selector rules look at the
  *shape* of the request (key, headers, tags, prompt size, tools, images), which is deterministic and costs
  no extra call. For difficulty it has the `stage_router` (tool-result signals) and the `llm_classifier`
  (a judge model), both described in [routing](routing.md); there is no built-in scorer that reads the prompt
  text without a model call.
- **Where they overlap.** Both can keep image requests off targets that cannot take them, both move a
  request on when a prompt does not fit a window, both have a dry run, and both record why a decision was
  made.
- **Refusing instead of falling back.** humpyard's [strict routes](routing.md) refuse a request marked
  private rather than send it to a public provider. The LiteLLM page does not describe an equivalent.
- **Footprint.** humpyard is one static binary with a file for config; LiteLLM is a Python service, usually
  with a database.
- **Reach.** LiteLLM covers far more providers and is much more widely used.

## Choose humpyard when

- Your traffic is **coding agents**, which send a mix of tiny and huge requests, and you want the
  small ones on a cheap or free model and the tool loops on a capable one.
- You want to **ask what a request would do** before sending it, then read what it did and why.
- You want **one small binary with a file for config** and no database server or Python stack.
- Some requests must **stay on a private model** and the gateway should refuse to send them
  elsewhere ([strict routes](routing.md)).
- You are happy with **budgets that fail closed** and money handled as integers.

## Choose something else when

- You need **dozens of provider integrations with their own request formats**. humpyard talks to
  OpenAI-compatible and Anthropic-compatible endpoints.
- You want a **finished admin UI today**. Keys and routes are config in humpyard; a GUI is planned.
- You want **someone else to run it and bill you once**. A hosted router does that.
- You want a **built-in difficulty scorer that needs no model call**. LiteLLM's heuristic scorer does that;
  humpyard's difficulty routing uses a judge model or tool-result signals.
- You need a **mature, widely deployed project**. humpyard is [pre-alpha](roadmap.md): it runs real
  traffic, but expect rough edges.

## Moving over

- Coming from LiteLLM: [Migrating from LiteLLM](migrating-from-litellm.md) maps each setting and
  lists the verified differences.
- Want to try it first: [the no-keys demo](demo.md) takes about a minute.
- Questions: [Discord](https://discord.gg/qfTGr7KTfq).
