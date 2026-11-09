import { describe, expect, it } from "vitest";
import { buildRows, overallState, signalRate } from "./models";
import { group, health, stats } from "./fixtures";

describe("joining statistics and health", () => {
  it("keeps a configured target that has no calls, and a called target that is no longer configured", () => {
    const rows = buildRows(stats(group("old")), health({ name: "new", states: ["healthy"] }));
    expect(rows.map((r) => [r.target, r.stats === null, r.endpoints.length])).toEqual([
      ["old", false, 0],
      ["new", true, 1],
    ]);
  });

  it("orders by requests, busiest first, then by name", () => {
    const rows = buildRows(
      stats(group("b", { requests: 5 }), group("a", { requests: 5 }), group("c", { requests: 50 })),
      health(),
    );
    expect(rows.map((r) => r.target)).toEqual(["c", "a", "b"]);
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
