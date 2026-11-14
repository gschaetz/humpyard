import type { Baseline, BaselineComparison, Config, Health, Series, SeriesBucket, Stats, StatsGroup } from "./api/client";

export function group(name: string, over: Partial<StatsGroup> = {}): StatsGroup {
  return {
    group: name,
    requests: 10,
    attempts: 12,
    failures: 2,
    fallbacks: 1,
    answered: 9,
    outcomes: {},
    tokens: { input: 1000, cached_input: 0, cache_creation: 0, output: 500 },
    response_signals: { observed: 8, empty_output: 2, truncated_output: 0, bad_tool_call: 1, cut_stream: 0 },
    failure_rate: 0.1667,
    fallback_rate: 0.1,
    latency_ms: { p50: 400, p90: 900, p99: 2000 },
    avg_cost_usd: 0.002,
    avg_tokens: 150,
    ...over,
  };
}

export function stats(...data: StatsGroup[]): Stats {
  return { object: "stats", computed_at_ms: 1_700_000_000_000, group_by: "target", window: {}, truncated: false, data };
}

export function health(...targets: { name: string; states: ("healthy" | "cooling_down" | "probing")[]; latency?: number | null }[]): Health {
  return {
    targets: targets.map((t) => ({
      target: t.name,
      current_latency_ms: t.latency ?? null,
      latency_samples: t.latency == null ? null : 3,
      endpoints: t.states.map((state) => ({
        provider: "p",
        model: "m",
        state,
        consecutive_failures: 0,
        cooldown_remaining_ms: null,
        cooldown_source: null,
      })),
    })),
    config: { fingerprint: "abc", loaded_at_ms: 1 },
    reload: { attempts: 0, at_ms: null, ok: null, message: null },
  };
}


export const BUCKET_MS = 900_000;
export const T0 = Date.UTC(2026, 10, 12, 12, 0);

export function bucket(i: number, over: Partial<SeriesBucket> = {}): SeriesBucket {
  return {
    start_ms: T0 + i * BUCKET_MS,
    requests: 10,
    attempts: 10,
    failures: 1,
    failure_rate: 0.1,
    fallbacks: 0,
    fallback_rate: 0,
    latency_ms: { p50: 100, p90: 300, p99: 900 },
    ...over,
  };
}

/** A bucket in which the group had no requests: zero counts and null rates and percentiles. */
export function quietBucket(i: number): SeriesBucket {
  return bucket(i, { requests: 0, attempts: 0, failures: 0, failure_rate: null, fallback_rate: null, latency_ms: { p50: null, p90: null, p99: null } });
}

export function series(groups: Record<string, SeriesBucket[]>, over: Partial<Series> = {}): Series {
  return {
    object: "stats_series",
    computed_at_ms: T0,
    group_by: "target",
    interval_ms: BUCKET_MS,
    window: {},
    now_ms: T0,
    truncated: false,
    covered_since_ms: null,
    omitted_groups: 0,
    data: Object.entries(groups).map(([group, buckets]) => ({ group, buckets })),
    ...over,
  };
}

/** A configuration with these endpoints, as `provider/model` with a price (`null`: no price), under one target each. */
export function config(
  ...endpoints: { name: string; price: { input: number; output: number; cached_input?: number | null; tiers?: unknown[] } | null }[]
): Config {
  const targets: Config["targets"] = {};
  endpoints.forEach((e, i) => {
    const [provider = "", ...model] = e.name.split("/");
    targets[`t${i.toString()}`] = { endpoints: [{ provider, model: model.join("/"), price: e.price }] };
  });
  return { targets };
}

/** A statistics answer that was asked for a baseline: the baseline, its summary and no change to the groups. */
export function withBaseline(
  stats: Stats,
  over: { endpoint?: string; summary?: Partial<BaselineComparison> } = {},
): Stats & { baseline: Baseline; summary: BaselineComparison } {
  const summary: BaselineComparison = {
    baseline_cost_usd: 0.018,
    cost_usd: 0.012,
    saved_usd: 0.006,
    saved_ratio: 0.3333,
    calls_priced: 8,
    calls_left_out: 0,
    ...over.summary,
  };
  return {
    ...stats,
    baseline: {
      endpoint: over.endpoint ?? "p/big",
      input_per_million_usd: 3,
      cached_input_per_million_usd: 3,
      output_per_million_usd: 15,
      basis: "An estimate, not a measurement: the tokens these calls used, priced at this endpoint's prices.",
    },
    summary,
  };
}
