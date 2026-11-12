import { cleanup, fireEvent, render } from "@testing-library/preact";
import { afterEach, describe, expect, it, vi } from "vitest";
import { LineChart } from "./chart";
import { niceTicks, strokes, utc, type Point } from "./chart-data";
import { BUCKET_MS, T0, bucket, quietBucket, series } from "./fixtures";

afterEach(cleanup);

const MINUTE = BUCKET_MS;
const quiet = quietBucket;

const none = () => undefined;

describe("the line chart", () => {
  it("draws one line per group with a hover text for every point", () => {
    const { container } = render(
      <LineChart series={series({ a: [0, 1, 2].map((i) => bucket(i)), b: [0, 1, 2].map((i) => bucket(i)) })} metric="failure_rate" focus="" onFocus={none} />,
    );
    expect(container.querySelectorAll("g.line")).toHaveLength(2);
    const titles = [...container.querySelectorAll("circle.hit title")].map((t) => t.textContent);
    expect(titles).toHaveLength(6);
    expect(titles[0]).toBe("2026-11-12 12:00 UTC · a: Failure rate 10% (10 requests)");
  });

  it("leaves a gap, not a zero, for a quiet bucket, and says there were no calls on hover", () => {
    const { container } = render(
      <LineChart
        series={series({ a: [bucket(0), bucket(1), quiet(2), bucket(3), bucket(4), bucket(5)] })}
        metric="failure_rate"
        focus=""
        onFocus={none}
      />,
    );
    const paths = [...container.querySelectorAll("path.stroke-line:not(.partial)")];
    expect(paths).toHaveLength(2); // before and after the gap
    const titles = [...container.querySelectorAll("circle.hit title")].map((t) => t.textContent);
    expect(titles[2]).toBe("2026-11-12 12:30 UTC · a: no calls");
    expect(container.querySelectorAll(".mark").length).toBeGreaterThan(0);
  });

  it("draws the last, still filling bucket dashed", () => {
    const { container } = render(<LineChart series={series({ a: [0, 1, 2, 3].map((i) => bucket(i)) })} metric="requests" focus="" onFocus={none} />);
    expect(container.querySelectorAll("path.partial")).toHaveLength(1);
    expect(container.querySelectorAll("path.stroke-line:not(.partial)")).toHaveLength(1);
  });

  it("shades the part the row cap cut off and says so, and not otherwise", () => {
    const buckets = [0, 1, 2, 3, 4].map((i) => bucket(i));
    const cut = render(
      <LineChart series={series({ a: buckets }, { truncated: true, covered_since_ms: T0 + 2 * MINUTE })} metric="requests" focus="" onFocus={none} />,
    );
    expect(cut.container.querySelectorAll("rect.unread")).toHaveLength(1);
    expect(cut.container.textContent).toContain("not shown");
    expect(cut.getByRole("status").textContent).toContain("only the newest decisions");
    cleanup();
    const whole = render(<LineChart series={series({ a: buckets })} metric="requests" focus="" onFocus={none} />);
    expect(whole.container.querySelectorAll("rect.unread")).toHaveLength(0);
  });

  it("tells series apart by marker shape and direct label as well as colour", () => {
    const { container } = render(
      <LineChart series={series({ a: [bucket(0), bucket(1)], b: [bucket(0), bucket(1)], c: [bucket(0), bucket(1)] })} metric="requests" focus="" onFocus={none} />,
    );
    const shapes = [...container.querySelectorAll("g.line")].map((g) => g.querySelector(".mark")?.tagName);
    expect(new Set(shapes).size).toBe(3); // circle, rect, polygon
    expect([...container.querySelectorAll("text.label")].map((t) => t.textContent)).toEqual(["a", "b", "c"]);
  });

  it("filters to a series when it is chosen in the legend, and back when chosen again", () => {
    const onFocus = vi.fn();
    const data = series({ a: [bucket(0), bucket(1)], b: [bucket(0), bucket(1)] });
    const { getByRole, container, rerender } = render(<LineChart series={data} metric="requests" focus="" onFocus={onFocus} />);
    fireEvent.click(getByRole("button", { name: "b" }));
    expect(onFocus).toHaveBeenLastCalledWith("b");
    rerender(<LineChart series={data} metric="requests" focus="b" onFocus={onFocus} />);
    expect(container.querySelectorAll("g.line.dim")).toHaveLength(1);
    expect(getByRole("button", { name: "b" }).getAttribute("aria-pressed")).toBe("true");
    fireEvent.click(getByRole("button", { name: "b" }));
    expect(onFocus).toHaveBeenLastCalledWith("");
  });

  it("renders hostile group names as text and creates no element, attribute or style from them", () => {
    const hostile = "<img src=x onerror=alert(1)>";
    const { container, getAllByText } = render(
      <LineChart series={series({ [hostile]: [bucket(0), bucket(1)] })} metric="requests" focus={hostile} onFocus={none} />,
    );
    expect(getAllByText(hostile).length).toBeGreaterThan(0);
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelectorAll("[onerror]")).toHaveLength(0);
    expect(container.querySelectorAll("title")[0]?.textContent).toContain(hostile);
    expect(container.querySelectorAll("[style]")).toHaveLength(0);
  });

  it("writes no style attribute and no script or style element for any metric", () => {
    for (const metric of ["failure_rate", "fallback_rate", "p90", "requests"] as const) {
      const { container } = render(
        <LineChart
          series={series({ a: [bucket(0), quiet(1), bucket(2)] }, { truncated: true, covered_since_ms: T0 + MINUTE })}
          metric={metric}
          focus="a"
          onFocus={none}
        />,
      );
      expect(container.querySelectorAll("[style], script, style")).toHaveLength(0);
      cleanup();
    }
  });

  it("says so when there is nothing to draw", () => {
    const { container } = render(<LineChart series={series({})} metric="requests" focus="" onFocus={none} />);
    expect(container.textContent).toContain("No requests in this window");
    expect(container.querySelector("svg")).toBeNull();
  });
});

describe("the chart's arithmetic", () => {
  it("picks round tick values from zero up to at least the data", () => {
    expect(niceTicks(0)).toEqual([0, 0.5, 1]);
    expect(niceTicks(0.37)).toEqual([0, 0.1, 0.2, 0.3, 0.4]);
    for (const max of [0.004, 0.37, 1, 870, 12_345]) {
      const ticks = niceTicks(max);
      expect(ticks[0]).toBe(0);
      expect(Math.max(...ticks)).toBeGreaterThanOrEqual(max);
      expect(ticks.length).toBeLessThanOrEqual(7);
    }
  });

  it("splits strokes at a gap and keeps a lone point as a marker only", () => {
    const point = (x: number, y: number | null): Point => ({ start_ms: x, requests: 1, value: y, x, y });
    const { solid, partial } = strokes([point(0, 1), point(1, 2), point(2, null), point(3, 4), point(4, null), point(5, 6)]);
    expect(solid).toHaveLength(1);
    expect(partial).toBeNull();
  });

  it("names a time in UTC to the minute", () => {
    expect(utc(Date.UTC(2026, 0, 2, 3, 4, 59))).toBe("2026-01-02 03:04 UTC");
  });
});
