import type { Health, Stats, StatsGroup } from "./api/client";

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
  return { object: "stats", group_by: "target", window: {}, truncated: false, data };
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

