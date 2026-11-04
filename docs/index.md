# humpyard

**A gateway that sorts each LLM request onto the right model.** One small Rust binary that speaks
OpenAI Chat, OpenAI Responses and Anthropic Messages, routes with
[NVIDIA NeMo Switchyard](https://github.com/NVIDIA-NeMo/Switchyard) algorithms, fails over across
providers, and keeps virtual keys, budgets and a usage ledger in SQLite.

> **Using it, or thinking about it?** Come say hello on [Discord](https://discord.gg/qfTGr7KTfq): ask a question, share your
> setup, tell us what to build next.

*Why the name:* a hump yard is the railroad yard where cars are classified and sent down the right
track, one by one. That is the job here, with requests and models.

## What makes it different

Not a general proxy for every provider. humpyard is the **routing and policy layer for agent
traffic** (Claude Code, Codex and the like), and it explains and measures what it does:

- **Content-aware routing.** [Selector rules](routing.md#selecting-the-route) pick the route from the key,
  headers, tags, agent metadata, prompt size, tools and images. Clients can narrow, never widen.
- **Explainable.** `POST /v1/route/explain` says what a request would do, and why, before you send it.
- **Recorded and measured.** Every decision is kept (rule, attempts, fallbacks, refusals) and
  summarized as failure mix, latency percentiles and cost per route, target and endpoint.
- **Operable.** A single static binary or a small container, hot config reload, per-endpoint
  circuit breakers, graceful shutdown, and money handled as integers that fail closed.

## Start here

| I want to... | Read |
|---|---|
| see it work in a minute, no keys | [Try it](demo.md) |
| run it | [Deployment](deployment.md) |
| point Claude Code, Codex or OpenClaw at it | [Clients](clients.md) |
| route one `auto` model to the right pool | [Auto routing recipe](routing.md#auto-routing-recipe) |
| understand failover, selectors, health | [Routing](routing.md) |
| set budgets and read the usage and decision log | [Keys, budgets and the ledger](budgets.md) |
| cut what a coding agent costs | [Cut agent costs](cut-agent-costs.md) |
| compare it with other gateways | [Compared with other gateways](compare.md) |
| move from LiteLLM | [Migrating from LiteLLM](migrating-from-litellm.md) |
| move from ModelRelay | [Moving from ModelRelay](migrating-from-modelrelay.md) |
| see the design | [Architecture](architecture.md) and the [decision records](adr/README.md) |
| see what is done and what is next | [Roadmap](roadmap.md) |

Source, issues and releases: [github.com/gschaetz/humpyard](https://github.com/gschaetz/humpyard).
Questions and feedback: [Discord](https://discord.gg/qfTGr7KTfq). Please never paste API keys or full configs in chat.
Apache-2.0.
