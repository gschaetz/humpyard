import { describe, expect, it } from "vitest";
import { DEFAULT_QUERY, INTERVALS, MAX_TEXT, REFRESH_MS, WINDOWS, ZONES, parseQuery, serialiseQuery, type Query } from "./query";

describe("the query", () => {
  it("starts from a 24-hour window grouped by target on failure rate, refreshing every minute", () => {
    expect(DEFAULT_QUERY).toMatchObject({ window: "24h", group: "target", metric: "failure_rate", refresh: "1m" });
    expect(parseQuery("")).toEqual(DEFAULT_QUERY);
    expect(parseQuery("#")).toEqual(DEFAULT_QUERY);
  });

  it("round-trips every window, grouping, metric and refresh period", () => {
    for (const window of WINDOWS) {
      for (const group of ["route", "target", "endpoint"] as const) {
        for (const metric of ["failure_rate", "fallback_rate", "p90", "requests"] as const) {
          for (const refresh of ["off", "30s", "1m", "5m"] as const) {
            const query: Query = { ...DEFAULT_QUERY, window, group, metric, refresh };
            expect(parseQuery(serialiseQuery(query))).toEqual(query);
            expect(parseQuery(`#${serialiseQuery(query)}`)).toEqual(query);
          }
        }
      }
    }
  });

  it("starts in local time, round-trips both choices and writes only the one that is not the default", () => {
    expect(DEFAULT_QUERY.zone).toBe("local");
    expect(parseQuery("zone=local").zone).toBe("local");
    expect(parseQuery("zone=utc").zone).toBe("utc");
    for (const zone of ZONES) {
      const query: Query = { ...DEFAULT_QUERY, zone };
      expect(parseQuery(serialiseQuery(query))).toEqual(query);
    }
    expect(serialiseQuery({ ...DEFAULT_QUERY, zone: "utc" })).toContain("zone=utc");
    expect(serialiseQuery({ ...DEFAULT_QUERY, zone: "local" })).not.toContain("zone");
  });

  it("round-trips filters and a chosen series as plain strings, whatever characters they hold", () => {
    const query: Query = { ...DEFAULT_QUERY, route: "my route", target: "a&b=c#d", focus: "<img src=x onerror=alert(1)>" };
    expect(parseQuery(serialiseQuery(query))).toEqual(query);
  });

  it("leaves out an empty filter", () => {
    expect(serialiseQuery(DEFAULT_QUERY)).toBe("window=24h&group=target&metric=failure_rate&refresh=1m");
  });

  it("falls back to the defaults for values outside its lists", () => {
    const hostile = parseQuery("window=<script>alert(1)</script>&metric=../x&refresh=1ms&group=%27%3B&zone=../x");
    expect(hostile).toEqual(DEFAULT_QUERY);
  });

  it("treats an unknown refresh period such as 1ms as the default, and never a faster one", () => {
    expect(parseQuery("refresh=1ms").refresh).toBe("1m");
    expect(parseQuery("refresh=0").refresh).toBe("1m");
    expect(parseQuery("refresh=1s").refresh).toBe("1m");
    expect(REFRESH_MS["30s"]).toBe(30_000);
    expect(Math.min(...Object.values(REFRESH_MS).filter((ms) => ms > 0))).toBe(30_000);
  });

  it("keeps a filter as a string and drops one too long to be a name", () => {
    expect(parseQuery("target=%3Cscript%3E").target).toBe("<script>");
    expect(parseQuery(`target=${"x".repeat(MAX_TEXT)}`).target).toHaveLength(MAX_TEXT);
    expect(parseQuery(`target=${"x".repeat(MAX_TEXT + 1)}`).target).toBe("");
  });

  it("reads the time zone choice only when it is exactly one of the two", () => {
    expect(parseQuery("zone=UTC").zone).toBe("local");
    expect(parseQuery("zone=America%2FChicago").zone).toBe("local");
    expect(parseQuery("zone=utc%20").zone).toBe("local");
  });

  it("reads a list value only when it is exactly one of the list", () => {
    expect(parseQuery("window=7d%20").window).toBe("24h");
    expect(parseQuery("window=7D").window).toBe("24h");
    expect(parseQuery("window=7d").window).toBe("7d");
    expect(parseQuery("window=7d&window=1h").window).toBe("7d");
  });

  it("never writes, or reads back, anything resembling a credential or a token", () => {
    const parsed = parseQuery("token=abc&credential=hunter2&authorization=Bearer%20x&access_token=y&window=7d");
    expect(Object.keys(parsed).sort()).toEqual(["focus", "group", "metric", "refresh", "route", "target", "window", "zone"]);
    const written = serialiseQuery(parsed);
    expect(written).not.toMatch(/token|credential|authorization|bearer|hunter2|abc/i);
    expect(written).toContain("window=7d");
  });

  it("asks for a bucket width that keeps a chart between 60 and 120 buckets", () => {
    const minutes = { "1m": 1, "5m": 5, "15m": 15, "2h": 120, "6h": 360 } as const;
    const windowMinutes = { "1h": 60, "6h": 360, "24h": 1440, "7d": 10080, "30d": 43200 } as const;
    for (const window of WINDOWS) {
      const buckets = windowMinutes[window] / minutes[INTERVALS[window] as keyof typeof minutes];
      expect(buckets).toBeGreaterThanOrEqual(60);
      expect(buckets).toBeLessThanOrEqual(120);
    }
  });
});
