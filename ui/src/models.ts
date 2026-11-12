import type { EndpointHealth, Health, Stats, StatsGroup } from "./api/client";

export { WINDOWS, type Window } from "./query";

export const SIGNALS = ["empty_output", "truncated_output", "bad_tool_call", "cut_stream"] as const;
export type Signal = (typeof SIGNALS)[number];

/** One target as the models view shows it: what the window's statistics say, and how its endpoints are now. */
export interface ModelRow {
  target: string;
  stats: StatsGroup | null;
  endpoints: EndpointHealth[];
  /** The smoothed time to answer routes see now; null until the target has answered. */
  currentLatencyMs: number | null;
}

/**
 * A target that has endpoints but no calls in the window is still a row (with no statistics), and a target that was
 * called but is no longer configured is one too (with no endpoints), so nothing in either answer is dropped.
 */
export function buildRows(stats: Stats, health: Health): ModelRow[] {
  const byTarget = new Map<string, ModelRow>();
  for (const target of health.targets) {
    byTarget.set(target.target, {
      target: target.target,
      stats: null,
      endpoints: target.endpoints,
      currentLatencyMs: target.current_latency_ms,
    });
  }
  for (const group of stats.data) {
    const row = byTarget.get(group.group);
    if (row) row.stats = group;
    else byTarget.set(group.group, { target: group.group, stats: group, endpoints: [], currentLatencyMs: null });
  }
  return [...byTarget.values()].sort((a, b) => (b.stats?.requests ?? 0) - (a.stats?.requests ?? 0) || a.target.localeCompare(b.target));
}

/** The share of the calls whose signals were observed that carried `signal`; null when none were observed. */
export function signalRate(group: StatsGroup, signal: Signal): number | null {
  const { observed } = group.response_signals;
  return observed > 0 ? group.response_signals[signal] / observed : null;
}

/** The worst endpoint state of a target, for a one-word summary. */
export function overallState(endpoints: EndpointHealth[]): "healthy" | "probing" | "cooling_down" | "none" {
  if (endpoints.length === 0) return "none";
  if (endpoints.every((e) => e.state === "cooling_down")) return "cooling_down";
  if (endpoints.some((e) => e.state !== "healthy")) return "probing";
  return "healthy";
}
