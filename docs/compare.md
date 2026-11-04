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
| Routing | selector rules on key, headers, tags, prompt size, tools, images; `stage_router`, `fastest`, `random`, pools | model groups, fallbacks, load balancing | vendor-side model choice |
| Dry run | `POST /v1/route/explain` shows the decision before any call | not a core feature | not applicable |
| Keys and budgets | hashed virtual keys in config, daily and monthly caps, per-request ceiling | virtual keys and budgets, usually in a database | account credits |
| Usage record | SQLite ledger, one row per decision with attempts and fallbacks | spend logs | dashboard |
| Your prompts leave your network | only to the providers you configure | only to the providers you configure | also through the router |
| Admin UI | not yet ([planned](roadmap.md#planned-the-operator-surface)) | yes | yes |

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
- You need a **mature, widely deployed project**. humpyard is [pre-alpha](roadmap.md): it runs real
  traffic, but expect rough edges.

## Moving over

- Coming from LiteLLM: [Migrating from LiteLLM](migrating-from-litellm.md) maps each setting and
  lists the verified differences.
- Want to try it first: [the no-keys demo](demo.md) takes about a minute.
- Questions: [Discord](https://discord.gg/qfTGr7KTfq).
