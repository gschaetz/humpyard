import type { Series, SeriesBucket } from "./api/client";
import { count, ms, percent } from "./format";
import type { Metric } from "./query";

/** The most series drawn: also the gateway's default `limit`, and the number of distinct colours and shapes. */
export const MAX_SERIES = 8;

export const WIDTH = 640;
export const HEIGHT = 280;
export const LEFT = 52;
export const RIGHT = 108;
export const TOP = 12;
export const BOTTOM = 28;

/** A bucket's value for the metric; null when the bucket has none (a quiet bucket), which is a gap and never a zero. */
export function bucketValue(bucket: SeriesBucket, metric: Metric): number | null {
  switch (metric) {
    case "failure_rate":
      return bucket.failure_rate;
    case "fallback_rate":
      return bucket.fallback_rate;
    case "p90":
      return bucket.latency_ms.p90;
    case "requests":
      return bucket.requests;
  }
}

export function formatMetric(metric: Metric, value: number | null): string {
  if (value === null) return "no value";
  if (metric === "p90") return ms(value);
  if (metric === "requests") return count(value);
  return percent(value);
}

export const METRIC_LABELS: Record<Metric, string> = {
  failure_rate: "Failure rate",
  fallback_rate: "Fallback rate",
  p90: "Latency p90",
  requests: "Requests",
};

/** UTC, to the minute, so a point's text is the same wherever the page is read. */
export function utc(startMs: number): string {
  return `${new Date(startMs).toISOString().slice(0, 16).replace("T", " ")} UTC`;
}

/** Tick values from 0 up to a round number at or above `max`, about `want` of them. */
export function niceTicks(max: number, want = 4): number[] {
  const top = max > 0 ? max : 1;
  const rough = top / want;
  const magnitude = 10 ** Math.floor(Math.log10(rough));
  const step = ([1, 2, 5, 10].find((m) => m * magnitude >= rough) ?? 10) * magnitude;
  const ticks: number[] = [];
  for (let value = 0; value < top + step * 0.999; value += step) ticks.push(Number(value.toPrecision(12)));
  return ticks;
}

export interface Point {
  start_ms: number;
  requests: number;
  value: number | null;
  x: number;
  /** null when there is no value: nothing is drawn there. */
  y: number | null;
}

export interface Line {
  group: string;
  points: Point[];
  /** Where the label at the end of the line is drawn: its last value's height, nudged so labels do not overlap. */
  labelY: number | null;
}

const LABEL_GAP = 13;

export interface Layout {
  lines: Line[];
  yTicks: { value: number; y: number }[];
  xTicks: { x: number; label: string }[];
  /** The right edge of the unread region, when the answer says the row cap cut the oldest requests; else null. */
  unreadTo: number | null;
}

const plotWidth = WIDTH - LEFT - RIGHT;
const plotHeight = HEIGHT - TOP - BOTTOM;

/** Scales for a series and a metric. All geometry is computed here and written as plain SVG attributes. */
export function layout(series: Series, metric: Metric): Layout {
  const groups = series.data.slice(0, MAX_SERIES);
  const starts = groups[0]?.buckets.map((b) => b.start_ms) ?? [];
  const first = starts[0] ?? 0;
  const last = starts.at(-1) ?? first;
  const x = (start: number) => (last === first ? LEFT + plotWidth / 2 : LEFT + ((start - first) / (last - first)) * plotWidth);

  let max = 0;
  for (const g of groups) {
    for (const b of g.buckets) max = Math.max(max, bucketValue(b, metric) ?? 0);
  }
  const ticks = niceTicks(max);
  const yMax = ticks.at(-1) ?? 1;
  const y = (value: number) => TOP + plotHeight - (value / yMax) * plotHeight;

  const lines: Line[] = groups.map((g) => ({
    group: g.group,
    points: g.buckets.map((b) => {
      const value = bucketValue(b, metric);
      return { start_ms: b.start_ms, requests: b.requests, value, x: x(b.start_ms), y: value === null ? null : y(value) };
    }),
    labelY: null,
  }));
  spreadLabels(lines);

  const every = Math.max(1, Math.ceil(starts.length / 5));
  const long = last - first > 36 * 3_600_000;
  const xTicks = starts
    .filter((_, i) => i % every === 0)
    .map((start) => ({
      x: x(start),
      label: new Date(start).toISOString().slice(long ? 5 : 11, long ? 10 : 16),
    }));

  const covered = series.covered_since_ms;
  const unreadTo = series.truncated && covered !== null && covered > first ? Math.min(x(covered), LEFT + plotWidth) : null;
  return { lines, yTicks: ticks.map((value) => ({ value, y: y(value) })), xTicks, unreadTo };
}

/**
 * Sets each line's label height: the height of its last value, then pushed apart (top to bottom, then back up from the
 * bottom edge) so that labels of lines that end close together stay readable instead of printing over one another.
 */
function spreadLabels(lines: Line[]): void {
  const ends = lines
    .map((line) => ({ line, y: line.points.filter((p) => p.y !== null).at(-1)?.y ?? null }))
    .filter((e): e is { line: Line; y: number } => e.y !== null)
    .sort((a, b) => a.y - b.y);
  let previous = TOP - LABEL_GAP;
  for (const e of ends) {
    previous = Math.max(e.y, previous + LABEL_GAP);
    e.line.labelY = previous;
  }
  const floor = HEIGHT - BOTTOM + 4;
  let next = floor;
  for (const e of ends.reverse()) {
    if (e.line.labelY !== null && e.line.labelY > next) e.line.labelY = next;
    next = (e.line.labelY ?? next) - LABEL_GAP;
  }
}

const path = (points: Point[]) => points.map((p, i) => `${i === 0 ? "M" : "L"}${p.x.toFixed(1)} ${(p.y ?? 0).toFixed(1)}`).join(" ");

/**
 * The strokes of one line: runs of consecutive buckets with a value, each its own path so a quiet bucket is a gap.
 * The last bucket is still filling, so the stroke into it is `partial` (drawn dashed); a lone point is only a marker.
 */
export function strokes(points: Point[]): { solid: string[]; partial: string | null } {
  const runs: Point[][] = [];
  let run: Point[] = [];
  for (const p of points) {
    if (p.y === null) {
      if (run.length > 0) runs.push(run);
      run = [];
    } else {
      run.push(p);
    }
  }
  if (run.length > 0) runs.push(run);

  let partial: string | null = null;
  const lastPoint = points.at(-1);
  const final = runs.at(-1);
  if (final && lastPoint && final.at(-1) === lastPoint && final.length > 1) {
    partial = path(final.slice(-2));
    final.pop();
  }
  return { solid: runs.filter((r) => r.length > 1).map(path), partial };
}

/** A point's hover text: when, which series, the value and how many requests it rests on. */
export function pointTitle(group: string, metric: Metric, p: Point): string {
  const what = p.value === null ? "no calls" : `${METRIC_LABELS[metric]} ${formatMetric(metric, p.value)} (${count(p.requests)} requests)`;
  return `${utc(p.start_ms)} · ${group}: ${what}`;
}
