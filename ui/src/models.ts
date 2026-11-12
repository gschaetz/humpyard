import type { EndpointHealth, Health, Stats, StatsGroup } from "./api/client";
import type { Group } from "./query";

export { WINDOWS, type Window } from "./query";

export const SIGNALS = ["empty_output", "truncated_output", "bad_tool_call", "cut_stream"] as const;
export type Signal = (typeof SIGNALS)[number];

/** One group of the table: what the window's statistics say, and (for targets and endpoints) how it is now. */
export interface ModelRow {
  /** The route, target or `provider/model` the row stands for. */
  group: string;
  stats: StatsGroup | null;
  /** The endpoints behind the group; empty for a route, which has no endpoint state of its own. */
  endpoints: EndpointHealth[];
  /** Whether this grouping has health at all (a route does not), as opposed to a target with no endpoints. */
  hasHealth: boolean;
  /** The smoothed time to answer routes see now (targets only); null until the target has answered. */
  currentLatencyMs: number | null;
}

/**
 * The rows for a grouping. A group that is configured but has no calls in the window is still a row (with no
 * statistics), and one that was called but is no longer configured is one too (with no endpoints), so nothing in
 * either answer is dropped. Rows come back in the default, problems-first order.
 */
export function buildRows(stats: Stats, health: Health, group: Group = "target"): ModelRow[] {
  const rows = new Map<string, ModelRow>();
  const hasHealth = group !== "route";
  const add = (name: string, endpoints: EndpointHealth[], currentLatencyMs: number | null) => {
    const row = rows.get(name);
    if (row) {
      row.endpoints.push(...endpoints);
    } else {
      rows.set(name, { group: name, stats: null, endpoints: [...endpoints], hasHealth, currentLatencyMs });
    }
  };
  if (group === "target") {
    for (const target of health.targets) add(target.target, target.endpoints, target.current_latency_ms);
  } else if (group === "endpoint") {
    for (const target of health.targets) {
      for (const endpoint of target.endpoints) add(`${endpoint.provider}/${endpoint.model}`, [endpoint], null);
    }
  }
  for (const data of stats.data) {
    const row = rows.get(data.group);
    if (row) row.stats = data;
    else rows.set(data.group, { group: data.group, stats: data, endpoints: [], hasHealth, currentLatencyMs: null });
  }
  return sortRows([...rows.values()], { by: "default", descending: false });
}

/** The share of the calls whose signals were observed that carried `signal`; null when none were observed. */
export function signalRate(group: StatsGroup, signal: Signal): number | null {
  const { observed } = group.response_signals;
  return observed > 0 ? group.response_signals[signal] / observed : null;
}

export type State = "healthy" | "probing" | "cooling_down" | "none";

/** The worst endpoint state of a group, for a one-word summary. */
export function overallState(endpoints: EndpointHealth[]): State {
  if (endpoints.length === 0) return "none";
  if (endpoints.every((e) => e.state === "cooling_down")) return "cooling_down";
  if (endpoints.some((e) => e.state !== "healthy")) return "probing";
  return "healthy";
}

const Z = 1.96;

/**
 * The lower end of the 95% Wilson interval of `part` out of `whole`: how low the true rate can plausibly be. It
 * ranks a rate by how well it is backed, so 1 failure of 1 attempt (about 21%) sits just under 30 of 100 (about 22%),
 * and 1% of 10,000 far below both. No calls give 0.
 */
export function wilsonLower(part: number, whole: number): number {
  if (whole <= 0) return 0;
  const p = part / whole;
  const z2 = Z * Z;
  const centre = p + z2 / (2 * whole);
  const spread = Z * Math.sqrt((p * (1 - p)) / whole + z2 / (4 * whole * whole));
  return Math.max(0, (centre - spread) / (1 + z2 / whole));
}

export type SortKey = "default" | "group" | "health" | "requests" | "failure_rate" | "fallback_rate" | "latency";
export interface Sort {
  by: SortKey;
  descending: boolean;
}

/** 0 when every endpoint is cooling down, 1 when any is not healthy, 2 otherwise (and for a group with no health). */
function tier(row: ModelRow): number {
  const state = overallState(row.endpoints);
  return state === "cooling_down" ? 0 : state === "probing" ? 1 : 2;
}

function byName(a: ModelRow, b: ModelRow): number {
  return a.group.localeCompare(b.group);
}

/** Highest first. */
function high(a: number, b: number): number {
  return b - a;
}

/** The default order: groups that had calls before those that had none, then problems first (see the spec). */
function problemsFirst(a: ModelRow, b: ModelRow): number {
  const called = Number(b.stats !== null && b.stats.requests > 0) - Number(a.stats !== null && a.stats.requests > 0);
  if (called !== 0) return called;
  const as = a.stats;
  const bs = b.stats;
  return (
    tier(a) - tier(b) ||
    high(as ? wilsonLower(as.failures, as.attempts) : 0, bs ? wilsonLower(bs.failures, bs.attempts) : 0) ||
    high(as ? wilsonLower(as.fallbacks, as.requests) : 0, bs ? wilsonLower(bs.fallbacks, bs.requests) : 0) ||
    high(as?.requests ?? 0, bs?.requests ?? 0) ||
    byName(a, b)
  );
}

/** A column's raw value, or null when the row has none (those rows go last whichever way the column runs). */
function value(row: ModelRow, by: Exclude<SortKey, "default" | "group">): number | null {
  const g = row.stats;
  switch (by) {
    case "health":
      return row.hasHealth ? tier(row) : null;
    case "requests":
      return g?.requests ?? null;
    case "failure_rate":
      return g?.failure_rate ?? null;
    case "fallback_rate":
      return g?.fallback_rate ?? null;
    case "latency":
      return g?.latency_ms.p90 ?? null;
  }
}

/**
 * Orders rows. `default` is the problems-first order (`descending` is ignored); any other key sorts by the column's
 * raw values, ties by name, with rows that have no value last in either direction.
 */
export function sortRows(rows: ModelRow[], sort: Sort): ModelRow[] {
  const copy = [...rows];
  if (sort.by === "default") return copy.sort(problemsFirst);
  const direction = sort.descending ? -1 : 1;
  const by = sort.by;
  return copy.sort((a, b) => {
    if (by === "group") return direction * byName(a, b);
    const x = value(a, by);
    const y = value(b, by);
    if (x === null || y === null) return Number(x === null) - Number(y === null) || byName(a, b);
    return direction * (x - y) || byName(a, b);
  });
}
