/**
 * The query view's query: what the person asked to see, kept in the URL fragment so a view can be bookmarked and
 * shared. The fragment is untrusted input (anyone can send a link), so it is read against fixed lists: an unknown
 * window, grouping, metric or refresh period becomes the default, and a filter or focus value is only ever kept as a
 * plain string to be shown as text and sent as an encoded query parameter. Nothing here holds a credential or a token,
 * and none is ever written to the fragment.
 */

export const WINDOWS = ["1h", "6h", "24h", "7d", "30d"] as const;
export type Window = (typeof WINDOWS)[number];

export const GROUPS = ["route", "target", "endpoint"] as const;
export type Group = (typeof GROUPS)[number];

export const METRICS = ["failure_rate", "fallback_rate", "p90", "requests"] as const;
export type Metric = (typeof METRICS)[number];

/** The refresh periods on offer, and what each means in milliseconds (`off` never refreshes). */
export const REFRESH_MS = { off: 0, "30s": 30_000, "1m": 60_000, "5m": 300_000 } as const;
export type Refresh = keyof typeof REFRESH_MS;
export const REFRESHES = Object.keys(REFRESH_MS) as Refresh[];

/** The bucket width asked of the gateway for each window, which keeps a chart between 60 and 120 buckets. */
export const INTERVALS: Record<Window, string> = { "1h": "1m", "6h": "5m", "24h": "15m", "7d": "2h", "30d": "6h" };

/** The longest filter or focus value kept; a longer one is dropped rather than cut, so it never reads as another value. */
export const MAX_TEXT = 200;

export interface Query {
  window: Window;
  group: Group;
  metric: Metric;
  refresh: Refresh;
  /** Only this route; empty means every route. */
  route: string;
  /** Only attempts on this target; empty means every target. */
  target: string;
  /** The series chosen in the chart, which filters the table to it; empty means none. */
  focus: string;
}

export const DEFAULT_QUERY: Query = {
  window: "24h",
  group: "target",
  metric: "failure_rate",
  refresh: "1m",
  route: "",
  target: "",
  focus: "",
};

function oneOf<T extends string>(list: readonly T[], value: string | null, fallback: T): T {
  return list.find((item) => item === value) ?? fallback;
}

function text(value: string | null): string {
  return value !== null && value.length <= MAX_TEXT ? value : "";
}

/** Reads a fragment (with or without its leading `#`); anything not understood is the default. */
export function parseQuery(fragment: string): Query {
  const params = new URLSearchParams(fragment.startsWith("#") ? fragment.slice(1) : fragment);
  return {
    window: oneOf(WINDOWS, params.get("window"), DEFAULT_QUERY.window),
    group: oneOf(GROUPS, params.get("group"), DEFAULT_QUERY.group),
    metric: oneOf(METRICS, params.get("metric"), DEFAULT_QUERY.metric),
    refresh: oneOf(REFRESHES, params.get("refresh"), DEFAULT_QUERY.refresh),
    route: text(params.get("route")),
    target: text(params.get("target")),
    focus: text(params.get("focus")),
  };
}

/** The fragment for a query, without the leading `#`. Only the fields of a query are ever written. */
export function serialiseQuery(query: Query): string {
  const params = new URLSearchParams({
    window: query.window,
    group: query.group,
    metric: query.metric,
    refresh: query.refresh,
  });
  for (const name of ["route", "target", "focus"] as const) {
    if (query[name] !== "") params.set(name, query[name]);
  }
  return params.toString();
}
