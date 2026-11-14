---
description: Route Claude Code or Codex traffic so small requests use a cheap model and tool loops use a capable one, and measure what it saved.
---

# Cut the cost of coding-agent traffic

Coding agents send very different requests through one model name: a one-line title, a summary, a
60,000-token tool loop, a sub-agent doing narrow work. Sending all of it to your most capable model
is the expensive default. This page shows how to split it with humpyard and then check, from the
gateway's own ledger, whether the split saved anything.

## 1. See the idea first, with no keys

```sh
git clone https://github.com/gschaetz/humpyard && cd humpyard
HUMPYARD=/path/to/humpyard examples/demo/run.sh
```

It sends the same 11 requests twice, once all to one big model and once as `auto`, and compares
the cost from the ledger. (On your own traffic you do not need two keys: [ask for a baseline](#what-would-it-have-cost).) The tokens are from a mock provider and the percentage depends on the
demo's prices and mix, so treat it as a demonstration of the mechanism, not a promise ([what the
demo does not show](demo.md#what-it-does-not-show)).

## 2. Point your agent at the gateway

For Claude Code, give the small, cheap slot its own route and everything else the stage router
(settings and a verified run are on the [clients page](clients.md#claude-code-verified-2026-10-08-claude-code-2.1.285)):

```sh
export ANTHROPIC_BASE_URL=http://127.0.0.1:8080
export ANTHROPIC_API_KEY=sk-humpyard-...        # from `humpyard keygen`
export ANTHROPIC_MODEL=agent
export ANTHROPIC_DEFAULT_SONNET_MODEL=agent
export ANTHROPIC_DEFAULT_OPUS_MODEL=agent
export ANTHROPIC_DEFAULT_HAIKU_MODEL=fast       # titles, summaries
```

```toml
[routes.fast]
type = "passthrough"
targets = ["free-fast", "paid-fast"]            # free first, paid if the free tier fails

[routes.agent]
type = "stage_router"                           # efficient until tool errors, then capable
efficient = ["fast"]
capable = ["smart"]
```

[Codex and OpenClaw](clients.md) work the same way.

## 3. Put a ceiling on it

A runaway loop is the other way agent bills grow. Cap a key per day and per request
([budgets](budgets.md)):

```toml
[keys.me]
sha256 = "sha256:..."
daily_usd = 5.00
max_request_usd = 0.50
```

Budgets fail closed: a key over its cap is refused, not billed.

## 4. Measure, don't guess

```sh
humpyard ctl stats          # cost, tokens and latency per route, target and endpoint
curl localhost:8080/v1/stats -H "Authorization: Bearer $KEY"
```

### What would it have cost?

The quickest way to see whether the split saved anything is to ask the gateway to price the same traffic at the model you
would otherwise have used for everything. That needs an admin credential with the `read` scope
(`humpyard keygen ops --admin --scopes read`, see [the admin API](api.md#the-admin-api)) and the name of that model as
your config has it, `provider/model`:

```sh
curl "$GATEWAY/admin/v1/stats?window=7d&group_by=route&baseline=paid/capable-large" \
  -H "Authorization: Bearer $ADMIN"
```

Every group, and a `summary` of all of them, now says what its answered calls would have cost at that model's prices,
what they cost, and the difference. For the demo above (a mock provider and the demo's prices) it reads like this, with
the free route saving all of its baseline cost and the tool turns, which went to the baseline model itself, saving none:

```json
"summary": {"baseline_cost_usd": 0.01788, "cost_usd": 0.0123, "saved_usd": 0.00558, "saved_ratio": 0.312,
            "calls_priced": 8, "calls_left_out": 0}
```

Read it as what it is. It takes the tokens the calls used, not the tokens the baseline model would have used (another
model counts differently), and it says nothing about whether the baseline model would have answered as well; it is an
arithmetic comparison, and the answer states that in `baseline.basis`. Calls whose provider reported no usage are left out
and counted, and `saved_usd` is negative when the baseline would have been cheaper. Pick the baseline honestly: the model
you would really have used for everything. [The API guide](api.md#what-the-traffic-would-have-cost) has the details.

Compare a week on the split with a week without it on your own traffic. Small models are not
equal at agent work (see the note on model quality on the [clients page](clients.md)), so also watch
whether tasks still finish; if they do not, raise `quality` or `min_quality` on the pool.

Tell us what you find on [Discord](https://discord.gg/qfTGr7KTfq). Real numbers from real setups
are the most useful thing you can contribute.
