# Metrics and alerts

humpyard exposes operational metrics in the Prometheus text format, so the Prometheus and Grafana
you already run need no custom UI. They answer "is it healthy right now": request rate, errors,
failovers, latency, spend, endpoint breaker state, the ledger backlog and provider quota. The
ledger stays the source of truth for cost and history (`/v1/stats`, `/v1/decisions`); metrics are
live aggregates that reset when the process restarts, which Prometheus handles.
Every restart or rollout gives the metrics a new `pod` and `instance` label, so a gauge queried bare shows one
series per pod that ran in the range; the example dashboards aggregate (`max by (...)`, `sum(increase(...))`) for that reason, and
your own panels should too.

Metrics are **off by default**.

## Turn it on

The recommended setup is a **separate listener** for metrics. `/metrics` is then not on the public
port at all, so no ingress that routes to the gateway can reach it, and access is a network matter:

```toml
[metrics]
enabled = true
listen = "0.0.0.0:9100"   # a separate port; not the public `listen`
```

In a cluster, expose that port as a Service port named `metrics` and let only your monitoring
namespace reach it ([servicemonitor.yaml](https://github.com/gschaetz/humpyard/blob/main/examples/metrics/servicemonitor.yaml)
and [networkpolicy.yaml](https://github.com/gschaetz/humpyard/blob/main/examples/metrics/networkpolicy.yaml)).
No credential is needed there.

To serve it on the main listener instead, give it a credential only `/metrics` accepts:

```toml
[metrics]
enabled = true
bearer_sha256 = "sha256:..."   # from `humpyard keygen metrics`; the key itself goes in your scraper
```

Startup is refused if metrics are enabled on the main listener without a credential, or if the
credential equals a virtual key's. A virtual key cannot read `/metrics`, and the metrics credential
cannot call inference or any other endpoint.

## What is exposed

| Metric | Type | Labels |
|---|---|---|
| `humpyard_requests_total` | counter | `protocol`, `route`, `target`, `provider`, `status_class`, `key`\* |
| `humpyard_request_duration_seconds` | histogram | `route`, `target` |
| `humpyard_tokens_total` | counter | `route`, `target`, `direction` (`input`, `output`, `cached_input`, `cache_creation`), `key`\* |
| `humpyard_cost_micro_usd_total` | counter | `route`, `target`, `key`\* |
| `humpyard_failovers_total` | counter | `route`, `from_target`, `to_target` |
| `humpyard_refusals_total` | counter | `reason`, `key`\* |
| `humpyard_fallback_blocked_total` | counter | `route` |
| `humpyard_health_skips_total` | counter | `route`, `target` |
| `humpyard_retry_after_honored_total` | counter | `provider`, `source` (`body` today) |
| `humpyard_capability_skips_total` | counter | `route`, `target`, `reason` |
| `humpyard_target_latency_seconds` | gauge | `target` |
| `humpyard_discovery_targets` | gauge | (none; only when `[discovery] file` is set) |
| `humpyard_discovery_file_state` | gauge | `state` (`missing`, `empty`, `loaded`, `ignored`; 1 for the current state) |
| `humpyard_endpoint_state` | gauge | `target`, `provider`, `model`, `state` (1 for the current state) |
| `humpyard_ledger_queue_depth` | gauge | none |
| `humpyard_ledger_dropped_total`, `humpyard_ledger_failed_total` | counter | none |
| `humpyard_config_reload_total` | counter | `result` (`ok`, `rejected`) |
| `humpyard_config_last_reload_ok` | gauge | none |
| `humpyard_config_info` | gauge | `fingerprint` (the serving configuration's; one series, always 1) |
| `humpyard_config_loaded_timestamp_seconds` | gauge | none |
| `humpyard_overrides_active` | gauge | `kind` (`disable_target`, `drain_provider`, `pin_route`; all three always exported, 0 when none is active) |
| `humpyard_override_oldest_age_seconds` | gauge | none (how long the oldest [runtime override](routing.md#runtime-overrides) in force has been active; 0 when none is) |
| `humpyard_quota_percent` | gauge | `provider`, `window` |
| `humpyard_quota_snapshot_age_seconds` | gauge | `provider` |
| `humpyard_build_info` | gauge | `version` |

\* only with `key_labels = true`.

Notes:

- **Duration** is the time from a request's arrival to its response, which for a stream is the
  response headers; it is recorded only for requests that reached routing, so refusals do not skew it.
- **Cost** is at your configured prices, in millionths of a dollar, so `rate()` works and no float
  money is exposed. For spend you act on, use the ledger.
- **Blocked fallbacks** counts requests that failed upstream on a [strict route](routing.md#strict-routes)
  that had excluded targets: the fallback the route did not permit. It stays at 0 on routes that are not strict.
- **Health skips** counts requests in which a target was [left out of the plan](routing.md#endpoint-health)
  because every endpoint was cooling down and another target remained. It stays at 0 when nothing is cold, and
  when every eligible target is cold (the route fails open).
- **Capability skips** counts requests in which a target was [removed](routing.md#capabilities) because none
  of its endpoints could serve them, with `reason` `tools`, `vision` or `context`. Refusals with reason
  `no_capable_target` are in `refusals_total`, as are those with reason `override_no_target` (a
  [runtime override](routing.md#runtime-overrides) removed every target of the route).
- **A request that matched no route** is counted under `route="none"`.
- **Refusal reasons** are `unauthorized`, `forbidden`, `budget_exhausted`, `no_eligible_target`,
  `unknown_model`, `bad_request` and `no_capable_target`. Upstream failures show up in `status_class` and in failovers.

## What labels can contain

Label values come only from your configuration (route, target, provider and key names) or from small
fixed sets (protocol, status class, reason, state). A string a client sends, such as the model name
it asked for, a header or a tag, never becomes a label, so a client cannot grow the series or put
content into them. Nothing in `/metrics` is a prompt, a response or a secret.

**Per-key series are opt-in.** With `key_labels = true`, the configured key id is added to four
counters only: requests, tokens, cost and refusals, never to a histogram (buckets multiply with
routes, targets and keys). Only keys defined in your configuration can appear, and at most
`key_label_limit` (default 100) distinct values do; others are counted under `key="other"`.

```toml
[metrics]
enabled = true
listen = "0.0.0.0:9100"
key_labels = true
```

`[metrics]` needs a restart to change.

## Dashboards and alerts

Examples live in [`examples/metrics/`](https://github.com/gschaetz/humpyard/tree/main/examples/metrics)
and are attached to each release. They are examples to adapt, not part of the binary, and a test
checks that every metric they use exists, so a rename in the gateway breaks the build rather than
your dashboard.

- `grafana-dashboard.json`: the live view. Request rate, error ratio, latency percentiles,
  failovers, refusals, spend and tokens, spend by key (needs `key_labels`), unhealthy endpoints, quota
  and ledger backlog. Import it in Grafana and pick your Prometheus data source.
- `grafana-summary-dashboard.json`: totals instead of rates. For the time range you pick (30 days by
  default): requests, spend, cost per request, tokens, upstream error rate, p95 latency, failovers,
  refusals, quota, plus per-day bars and breakdowns by route, key, target, status class, failover path
  and refusal reason. It uses `increase()`, so a restart of humpyard does not break a total. It can
  only look back as far as your Prometheus retention, so keep at least 30 days (Prometheus defaults to 15)
  or pick a shorter range; for longer history use the ledger (`GET /v1/stats`).
- `alert-rules.yaml` and `prometheusrule.yaml` (the same rules for the Prometheus Operator): error
  ratio, failover rate, an endpoint cooling down, ledger drops, stale or high quota, a rejected
  config reload, and humpyard not being scraped. Thresholds are starting points.
- `servicemonitor.yaml` and `networkpolicy.yaml` for the recommended separate-listener setup.

## Not here yet

A read-only status page for people who do not run Prometheus is planned. It will read the same
registry, so it can never disagree with these numbers; see the [roadmap](roadmap.md).
