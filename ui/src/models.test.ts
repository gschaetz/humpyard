import { describe, expect, it } from "vitest";
import { buildRows, overallState, signalRate, sortRows, wilsonLower } from "./models";
import { group, health, stats } from "./fixtures";

describe("joining statistics and health", () => {
  it("keeps a configured target that has no calls, and a called target that is no longer configured", () => {
    const rows = buildRows(stats(group("old")), health({ name: "new", states: ["healthy"] }));
    expect(rows.map((r) => [r.group, r.stats === null, r.endpoints.length])).toEqual([
      ["old", false, 0],
      ["new", true, 1],
    ]);
  });

  it("gives an endpoint grouping its endpoints' own health, and a route grouping none", () => {
    const h = health({ name: "t", states: ["cooling_down"] });
    const endpoint = buildRows(stats(group("p/m")), h, "endpoint");
    expect(endpoint.map((r) => [r.group, r.hasHealth, r.endpoints.length])).toEqual([["p/m", true, 1]]);
    const route = buildRows(stats(group("r")), h, "route");
    expect(route.map((r) => [r.group, r.hasHealth, r.endpoints.length])).toEqual([["r", false, 0]]);
  });
});

const calls = (name: string, failures: number, attempts: number, over: Partial<Parameters<typeof group>[1]> = {}) =>
  group(name, { requests: attempts, attempts, failures, failure_rate: failures / attempts, ...over });

describe("the default order puts problems first", () => {
  it("lists a failing target above a much busier healthy one", () => {
    const rows = buildRows(stats(calls("busy", 100, 10_000), calls("failing", 3, 10)), health());
    expect(rows.map((r) => r.group)).toEqual(["failing", "busy"]);
  });

  it("does not let a lone failure outrank a well-sampled one, and shows both raw rates", () => {
    const rows = buildRows(stats(calls("lone", 1, 1), calls("sampled", 30, 100)), health());
    expect(rows.map((r) => r.group)).toEqual(["sampled", "lone"]);
    expect(rows.map((r) => r.stats?.failure_rate)).toEqual([0.3, 1]);
  });

  it("puts targets whose endpoints are all cooling down first, then those with any endpoint not healthy", () => {
    const rows = buildRows(
      stats(calls("failing", 9, 10), calls("one-down", 0, 10), calls("cold", 0, 10), calls("fine", 0, 10)),
      health(
        { name: "failing", states: ["healthy"] },
        { name: "one-down", states: ["healthy", "cooling_down"] },
        { name: "cold", states: ["cooling_down", "cooling_down"] },
        { name: "fine", states: ["healthy"] },
      ),
    );
    expect(rows.map((r) => r.group)).toEqual(["cold", "one-down", "failing", "fine"]);
  });

  it("lists a target with endpoints but no calls after every target that had calls, with no rate", () => {
    const rows = buildRows(
      stats(calls("called", 0, 5)),
      health({ name: "idle", states: ["cooling_down"] }, { name: "called", states: ["healthy"] }),
    );
    expect(rows.map((r) => r.group)).toEqual(["called", "idle"]);
    expect(rows[1]?.stats).toBeNull();
  });

  it("orders a route grouping by failure rate, without any health", () => {
    const rows = buildRows(stats(calls("calm", 0, 50), calls("flaky", 20, 50)), health({ name: "t", states: ["cooling_down"] }), "route");
    expect(rows.map((r) => r.group)).toEqual(["flaky", "calm"]);
    expect(rows.every((r) => !r.hasHealth)).toBe(true);
  });

  it("breaks ties by fallback rate, then requests, then name", () => {
    const quiet = { failures: 0, failure_rate: 0 };
    const rows = buildRows(
      stats(
        calls("b", 0, 10, { ...quiet, fallbacks: 0 }),
        calls("a", 0, 10, { ...quiet, fallbacks: 0 }),
        calls("c", 0, 20, { ...quiet, fallbacks: 0 }),
        calls("d", 0, 10, { ...quiet, fallbacks: 5 }),
      ),
      health(),
    );
    expect(rows.map((r) => r.group)).toEqual(["d", "c", "a", "b"]);
  });
});

describe("the interval ranks a rate by how well it is backed", () => {
  it("is zero with no calls and rises with the rate", () => {
    expect(wilsonLower(0, 0)).toBe(0);
    expect(wilsonLower(0, 10)).toBe(0);
    expect(wilsonLower(5, 10)).toBeGreaterThan(wilsonLower(1, 10));
  });

  it("is lower for the same rate on fewer calls", () => {
    expect(wilsonLower(3, 10)).toBeLessThan(wilsonLower(300, 1000));
    expect(wilsonLower(1, 1)).toBeLessThan(wilsonLower(30, 100));
    expect(wilsonLower(1, 1)).toBeGreaterThan(wilsonLower(100, 10_000));
  });

  it("never exceeds the raw rate", () => {
    for (const [part, whole] of [[1, 1], [3, 10], [30, 100], [0, 5], [5, 5]] as const) {
      expect(wilsonLower(part, whole)).toBeLessThanOrEqual(part / whole);
    }
  });
});

describe("sorting by a column", () => {
  const rows = () =>
    buildRows(
      stats(
        calls("a", 5, 10, { latency_ms: { p50: 1, p90: 300, p99: 9 } }),
        calls("b", 1, 100, { latency_ms: { p50: 1, p90: 100, p99: 9 } }),
        calls("c", 0, 10, { latency_ms: { p50: 1, p90: 200, p99: 9 } }),
      ),
      health({ name: "idle", states: ["healthy"] }),
    );

  it("follows the column's raw values, in either direction, ties by name", () => {
    expect(sortRows(rows(), { by: "requests", descending: true }).map((r) => r.group)).toEqual(["b", "a", "c", "idle"]);
    expect(sortRows(rows(), { by: "failure_rate", descending: false }).map((r) => r.group)).toEqual(["c", "b", "a", "idle"]);
    expect(sortRows(rows(), { by: "group", descending: true }).map((r) => r.group)).toEqual(["idle", "c", "b", "a"]);
  });

  it("leaves rows with no value last whichever way the column runs", () => {
    for (const descending of [false, true]) {
      expect(sortRows(rows(), { by: "latency", descending }).at(-1)?.group).toBe("idle");
    }
  });

  it("restores the default order", () => {
    const sorted = sortRows(rows(), { by: "group", descending: false });
    expect(sortRows(sorted, { by: "default", descending: true }).map((r) => r.group)).toEqual(rows().map((r) => r.group));
  });

  it("leaves its input alone", () => {
    const input = rows();
    const before = input.map((r) => r.group);
    sortRows(input, { by: "group", descending: true });
    expect(input.map((r) => r.group)).toEqual(before);
  });
});

describe("response-signal rates", () => {
  it("are counts over the calls that were observed", () => {
    expect(signalRate(group("t"), "empty_output")).toBe(0.25);
  });

  it("are null, not zero, when no call was observed", () => {
    const g = group("t", { response_signals: { observed: 0, empty_output: 0, truncated_output: 0, bad_tool_call: 0, cut_stream: 0 } });
    expect(signalRate(g, "empty_output")).toBeNull();
  });
});

describe("a target's overall state", () => {
  it.each([
    [[], "none"],
    [["healthy", "healthy"], "healthy"],
    [["healthy", "cooling_down"], "probing"],
    [["cooling_down", "cooling_down"], "cooling_down"],
    [["probing"], "probing"],
  ] as const)("%j is %s", (states, expected) => {
    const endpoints = health({ name: "t", states: [...states] }).targets[0]?.endpoints ?? [];
    expect(overallState(endpoints)).toBe(expected);
  });
});
