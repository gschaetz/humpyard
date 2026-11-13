import { cleanup, fireEvent, render } from "@testing-library/preact";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Bars } from "./bars";
import { formatMetric } from "./chart-data";
import { group, health, stats } from "./fixtures";
import { buildRows, metricOf } from "./models";

afterEach(cleanup);

const none = () => undefined;

function rows() {
  return buildRows(
    stats(
      group("worst", { requests: 10, attempts: 10, failures: 6, failure_rate: 0.6, fallback_rate: 0.2, latency_ms: { p50: 1, p90: 800, p99: 9 } }),
      group("best", { requests: 500, attempts: 500, failures: 0, failure_rate: 0, fallback_rate: 0, latency_ms: { p50: 1, p90: 120, p99: 9 } }),
    ),
    health({ name: "idle", states: ["healthy"] }),
  );
}

const names = (container: Element) => [...container.querySelectorAll("g.bar .bar-name")].map((t) => t.textContent.replace(/…$/, ""));

describe("the ranked bars", () => {
  it.each([
    ["failure_rate", ["worst", "best", "idle"]],
    ["fallback_rate", ["worst", "best", "idle"]],
    ["p90", ["worst", "best", "idle"]],
    ["requests", ["best", "worst", "idle"]],
  ] as const)("show %s highest first, with the group that has no value last", (metric, order) => {
    const { container } = render(<Bars rows={rows()} metric={metric} focus="" onFocus={none} />);
    expect(names(container)).toEqual(order);
    const shown = new Map(rows().map((r) => [r.group, formatMetric(metric, metricOf(r, metric))]));
    const values = [...container.querySelectorAll("g.bar .bar-value")].map((t) => t.textContent);
    values.forEach((text, i) => {
      expect(text.startsWith(shown.get(order[i] ?? "") ?? "?")).toBe(true);
    });
  });

  it("never draws a bar shorter than the one below it", () => {
    for (const metric of ["failure_rate", "fallback_rate", "p90", "requests"] as const) {
      const many = buildRows(
        stats(
          group("a", { requests: 40, attempts: 40, failures: 2, failure_rate: 0.05, fallback_rate: 0.3, latency_ms: { p50: 1, p90: 300, p99: 9 } }),
          group("b", { requests: 900, attempts: 900, failures: 90, failure_rate: 0.1, fallback_rate: 0.01, latency_ms: { p50: 1, p90: 100, p99: 9 } }),
          group("c", { requests: 5, attempts: 5, failures: 5, failure_rate: 1, fallback_rate: 0.9, latency_ms: { p50: 1, p90: 2000, p99: 9 } }),
          group("d", { requests: 300, attempts: 300, failures: 3, failure_rate: 0.01, fallback_rate: 0.05, latency_ms: { p50: 1, p90: 700, p99: 9 } }),
        ),
        health({ name: "a", states: ["cooling_down"] }),
      );
      const { container, unmount } = render(<Bars rows={many} metric={metric} focus="" onFocus={none} />);
      const widths = [...container.querySelectorAll("rect.bar-fill")].map((r) => Number(r.getAttribute("width")));
      expect(widths).toEqual([...widths].sort((x, y) => y - x));
      unmount();
    }
  });

  it("puts the slowest group first under latency whatever the health of any group", () => {
    const mixed = buildRows(
      stats(
        group("slow-but-healthy", { latency_ms: { p50: 1, p90: 5000, p99: 9 } }),
        group("fast-but-cooling", { latency_ms: { p50: 1, p90: 100, p99: 9 } }),
      ),
      health({ name: "slow-but-healthy", states: ["healthy"] }, { name: "fast-but-cooling", states: ["cooling_down"] }),
    );
    const { container } = render(<Bars rows={mixed} metric="p90" focus="" onFocus={none} />);
    expect(names(container)).toEqual(["slow-but-healthy", "fast-but-cooling"]);
  });

  it("breaks a tie by name", () => {
    const tied = buildRows(stats(group("b", { requests: 7 }), group("a", { requests: 7 })), health());
    const { container } = render(<Bars rows={tied} metric="requests" focus="" onFocus={none} />);
    expect(names(container)).toEqual(["a", "b"]);
  });

  it("places a rate from one call by its value and says it rests on one call", () => {
    const thin = buildRows(
      stats(
        group("steady", { requests: 500, attempts: 500, failures: 25, failure_rate: 0.05 }),
        group("lone", { requests: 1, attempts: 1, failures: 1, failure_rate: 1 }),
        group("few", { requests: 19, attempts: 19, failures: 9, failure_rate: 0.47 }),
        group("enough", { requests: 20, attempts: 20, failures: 8, failure_rate: 0.4 }),
      ),
      health(),
    );
    const { container } = render(<Bars rows={thin} metric="failure_rate" focus="" onFocus={none} />);
    expect(names(container)).toEqual(["lone", "few", "enough", "steady"]);
    const text = (name: string) => [...container.querySelectorAll("g.bar")].find((g) => g.querySelector(".bar-name")?.textContent === name)?.querySelector(".bar-value")?.textContent;
    expect(text("lone")).toBe("100% · 1 call");
    expect(text("few")).toContain("· 19 calls");
    expect(text("enough")).not.toContain("call");
    expect(text("steady")).not.toContain("call");
  });

  it("does not add a call count under the requests metric, where the value is that count", () => {
    const one = buildRows(stats(group("lone", { requests: 1, attempts: 1 })), health());
    const { container } = render(<Bars rows={one} metric="requests" focus="" onFocus={none} />);
    expect(container.querySelector(".bar-value")?.textContent).not.toContain("call");
    expect(container.querySelector(".bar-thin")).toBeNull();
  });

  it("shows a group without a value as no value, with no bar, not as zero", () => {
    const { container } = render(<Bars rows={rows()} metric="failure_rate" focus="" onFocus={none} />);
    const idle = [...container.querySelectorAll("g.bar")].find((g) => g.textContent.startsWith("idle"));
    expect(idle?.querySelector(".bar-value")?.textContent).toBe("no value");
    expect(idle?.querySelector("rect")).toBeNull();
    expect(container.textContent).not.toContain("0%no value");
  });

  it("scales the longest bar to the room and the others against it", () => {
    const { container } = render(<Bars rows={rows()} metric="requests" focus="" onFocus={none} />);
    const widths = [...container.querySelectorAll("rect.bar-fill")].map((r) => Number(r.getAttribute("width")));
    expect(widths).toHaveLength(2);
    // Listed longest first: "best" (500 requests, the whole 350 units of room) then "worst" (10 requests).
    expect(widths[0]).toBeCloseTo(350, 5);
    expect(widths[1]).toBeCloseTo(7, 5);
  });

  it("chooses a group on click and on Enter, and again to let go", () => {
    const onFocus = vi.fn();
    const { container, rerender } = render(<Bars rows={rows()} metric="requests" focus="" onFocus={onFocus} />);
    const first = container.querySelector("g.bar");
    fireEvent.click(first as Element);
    expect(onFocus).toHaveBeenLastCalledWith("best");
    fireEvent.keyDown(first as Element, { key: "Enter" });
    expect(onFocus).toHaveBeenCalledTimes(2);
    rerender(<Bars rows={rows()} metric="requests" focus="best" onFocus={onFocus} />);
    fireEvent.click(container.querySelector("g.bar") as Element);
    expect(onFocus).toHaveBeenLastCalledWith("");
    expect(container.querySelectorAll("g.bar.dim")).toHaveLength(2);
  });

  it("renders hostile names as text only, with no style attribute", () => {
    const hostile = "<img src=x onerror=alert(1)>";
    const { container } = render(<Bars rows={buildRows(stats(group(hostile)), health())} metric="requests" focus="" onFocus={none} />);
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelectorAll("[onerror], [style]")).toHaveLength(0);
    expect(container.textContent).toContain(hostile.slice(0, 10));
  });
});
