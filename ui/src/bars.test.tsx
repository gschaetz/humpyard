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

describe("the ranked bars", () => {
  it.each(["failure_rate", "fallback_rate", "p90", "requests"] as const)("show %s as the table's numbers, in the table's order", (metric) => {
    const ordered = rows();
    const { container } = render(<Bars rows={ordered} metric={metric} focus="" onFocus={none} />);
    const names = [...container.querySelectorAll("g.bar .bar-name")].map((t) => t.textContent.replace(/…$/, ""));
    expect(names).toEqual(ordered.map((r) => r.group));
    const values = [...container.querySelectorAll("g.bar .bar-value")].map((t) => t.textContent);
    expect(values).toEqual(ordered.map((r) => formatMetric(metric, metricOf(r, metric))));
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
    // Listed problems first: "worst" (10 requests) then "best" (500, the longest bar, the whole 400 units of room).
    expect(widths[0]).toBeCloseTo(8, 5);
    expect(widths[1]).toBeCloseTo(400, 5);
  });

  it("chooses a group on click and on Enter, and again to let go", () => {
    const onFocus = vi.fn();
    const { container, rerender } = render(<Bars rows={rows()} metric="requests" focus="" onFocus={onFocus} />);
    const first = container.querySelector("g.bar");
    fireEvent.click(first as Element);
    expect(onFocus).toHaveBeenLastCalledWith("worst");
    fireEvent.keyDown(first as Element, { key: "Enter" });
    expect(onFocus).toHaveBeenCalledTimes(2);
    rerender(<Bars rows={rows()} metric="requests" focus="worst" onFocus={onFocus} />);
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
