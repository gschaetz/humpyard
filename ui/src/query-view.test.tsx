import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/preact";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError, type Client } from "./api/client";
import { bucket, group, health, quietBucket, series, stats } from "./fixtures";
import { QueryView } from "./query-view";

const none = () => undefined;

function client(over: Partial<Client> = {}): Client {
  return {
    whoami: () => Promise.reject(new Error("not used")),
    contractVersion: () => Promise.reject(new Error("not used")),
    stats: () => Promise.resolve(stats(group("fast"))),
    series: () => Promise.resolve(series({ fast: [bucket(0), bucket(1), bucket(2)] })),
    health: () => Promise.resolve(health({ name: "fast", states: ["healthy"], latency: 350 })),
    exchangeToken: () => Promise.reject(new Error("not used")),
    ...over,
  };
}

function show(c: Client, onRefused = none) {
  return render(<QueryView client={c} onRefused={onRefused} />);
}

beforeEach(() => {
  window.history.replaceState(null, "", "#");
});
afterEach(cleanup);

/** The cells of the table row for a group. */
function cellsOf(name: string): string[] {
  const row = screen.getAllByText(name).map((e) => e.closest("tr")).find((r) => r !== null) as HTMLElement;
  return [...row.querySelectorAll("th, td")].map((c) => c.textContent);
}

describe("the query view's table", () => {
  it("leads with health, requests, failure rate, fallback rate and p90", async () => {
    show(client());
    await screen.findByRole("table");
    expect(cellsOf("fast")).toEqual(["▸", "fast", "healthy · 350 ms now", "10", "17%", "10%", "900 ms"]);
  });

  it("keeps tokens, cost, answered calls, percentiles and signal rates in an expandable detail row", async () => {
    show(client());
    await screen.findByRole("table");
    expect(screen.queryByText("Avg cost")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Details of fast" }));
    const detail = (await screen.findByText("Avg cost")).closest("dl") as HTMLElement;
    const text = detail.textContent;
    for (const expected of ["1.0k / 500", "$0.0020", "400 ms", "2,000 ms", "25%"]) expect(text).toContain(expected);
    fireEvent.click(screen.getByRole("button", { name: "Details of fast" }));
    expect(screen.queryByText("Avg cost")).toBeNull();
  });

  it("shows no signal rate, not 0%, for a target with no observed calls", async () => {
    const unobserved = group("quiet", { response_signals: { observed: 0, empty_output: 0, truncated_output: 0, bad_tool_call: 0, cut_stream: 0 } });
    show(client({ stats: () => Promise.resolve(stats(unobserved)) }));
    await screen.findByRole("table");
    fireEvent.click(screen.getByRole("button", { name: "Details of quiet" }));
    const detail = (await screen.findByText("empty output")).closest("dl") as HTMLElement;
    expect(within(detail).queryByText("0%")).toBeNull();
    expect(detail.textContent).toContain("empty output—");
  });

  it("lists a configured target with no calls without inventing figures", async () => {
    show(client({ stats: () => Promise.resolve(stats()), series: () => Promise.resolve(series({})), health: () => Promise.resolve(health({ name: "idle", states: ["healthy"] })) }));
    await screen.findByRole("table");
    expect(cellsOf("idle")).toEqual(["▸", "idle", "healthy", "0", "—", "—", "—"]);
  });

  it("renders a hostile name as text everywhere and creates no element, attribute or style from it", async () => {
    const hostile = "<img src=x onerror=alert(1)>";
    const { container } = show(
      client({
        stats: () => Promise.resolve(stats(group(hostile))),
        series: () => Promise.resolve(series({ [hostile]: [bucket(0), bucket(1)] })),
        health: () => Promise.resolve(health({ name: hostile, states: ["healthy"] })),
      }),
    );
    await screen.findByRole("table");
    expect(screen.getAllByText(hostile).length).toBeGreaterThan(0);
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelectorAll("[onerror], [style], script, style")).toHaveLength(0);
  });

  it("orders problems first and sorts by a column, with a way back", async () => {
    const two = stats(group("busy", { requests: 900, attempts: 900, failures: 9, failure_rate: 0.01 }), group("failing", { requests: 10, attempts: 10, failures: 4, failure_rate: 0.4 }));
    show(client({ stats: () => Promise.resolve(two), health: () => Promise.resolve(health()) }));
    await screen.findByRole("table");
    const names = () => screen.getAllByRole("rowheader").map((c) => c.textContent);
    expect(names()).toEqual(["failing", "busy"]);
    fireEvent.click(screen.getByRole("button", { name: /^Requests/ }));
    expect(names()).toEqual(["busy", "failing"]);
    fireEvent.click(screen.getByRole("button", { name: /^Default order/ }));
    expect(names()).toEqual(["failing", "busy"]);
    expect(screen.queryByRole("button", { name: /^Default order/ })).toBeNull();
  });

  it("shows no health column when grouped by route", async () => {
    window.history.replaceState(null, "", "#group=route");
    show(client({ stats: () => Promise.resolve(stats(group("r"))), series: () => Promise.resolve(series({ r: [bucket(0), bucket(1)] })), health: () => Promise.resolve(health()) }));
    await screen.findByRole("table");
    expect(screen.queryByRole("button", { name: /^Health/ })).toBeNull();
    expect(cellsOf("r")).toEqual(["▸", "r", "10", "17%", "10%", "900 ms"]);
  });
});

describe("the query bar and what is asked of the gateway", () => {
  it("starts from a 24-hour window grouped by target on failure rate, asking for series and statistics", async () => {
    const stat = vi.fn<Client["stats"]>().mockResolvedValue(stats(group("fast")));
    const ser = vi.fn<Client["series"]>().mockResolvedValue(series({ fast: [bucket(0), bucket(1)] }));
    show(client({ stats: stat, series: ser }));
    await screen.findByRole("table");
    expect(stat.mock.calls[0]?.[0]).toEqual({ window: "24h", groupBy: "target", route: undefined, target: undefined });
    expect(ser.mock.calls[0]?.[0]).toEqual({ window: "24h", interval: "15m", groupBy: "target", limit: 8, route: undefined, target: undefined });
    expect(screen.getByLabelText<HTMLSelectElement>("Metric").value).toBe("failure_rate");
  });

  it("reads both again for the new window, with the interval chosen for it", async () => {
    const stat = vi.fn<Client["stats"]>().mockResolvedValue(stats(group("fast")));
    const ser = vi.fn<Client["series"]>().mockResolvedValue(series({ fast: [bucket(0), bucket(1)] }));
    show(client({ stats: stat, series: ser }));
    await screen.findByRole("table");
    fireEvent.change(screen.getByLabelText("Window"), { target: { value: "7d" } });
    await waitFor(() => { expect(stat).toHaveBeenLastCalledWith(expect.objectContaining({ window: "7d" }), expect.anything()); });
    expect(ser).toHaveBeenLastCalledWith(expect.objectContaining({ window: "7d", interval: "2h" }), expect.anything());
  });

  it("says nothing matched, with no chart and no rows, when a filter matches nothing", async () => {
    window.history.replaceState(null, "", "#target=zzz");
    const ser = vi.fn<Client["series"]>().mockResolvedValue(series({}));
    show(client({ stats: () => Promise.resolve(stats()), series: ser, health: () => Promise.resolve(health({ name: "fast", states: ["healthy"] })) }));
    expect(await screen.findByText("Nothing matched this query.")).toBeTruthy();
    expect(screen.queryByRole("table")).toBeNull();
    expect(screen.queryByRole("img")).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(ser).toHaveBeenCalledWith(expect.objectContaining({ target: "zzz" }), expect.anything());
  });

  it("opens a shared link as that query, and writes a change back to the fragment without any credential", async () => {
    window.history.replaceState(null, "", "#window=7d&group=endpoint&metric=p90&target=fast");
    const stat = vi.fn<Client["stats"]>().mockResolvedValue(stats(group("p/m")));
    show(client({ stats: stat, series: () => Promise.resolve(series({ "p/m": [bucket(0), bucket(1)] })) }));
    await screen.findByRole("table");
    expect(stat).toHaveBeenCalledWith({ window: "7d", groupBy: "endpoint", target: "fast", route: undefined }, expect.anything());
    expect(screen.getByLabelText<HTMLSelectElement>("Window").value).toBe("7d");
    expect(screen.getByLabelText<HTMLSelectElement>("Group by").value).toBe("endpoint");
    expect(screen.getByLabelText<HTMLSelectElement>("Metric").value).toBe("p90");
    expect(screen.getByLabelText<HTMLInputElement>("Target").value).toBe("fast");
    fireEvent.change(screen.getByLabelText("Metric"), { target: { value: "requests" } });
    expect(window.location.hash).toBe("#window=7d&group=endpoint&metric=requests&refresh=1m&target=fast");
    expect(window.location.hash).not.toMatch(/token|credential|bearer|authorization/i);
  });

  it("uses the defaults for a hostile fragment and creates no element from it", async () => {
    window.history.replaceState(null, "", "#window=%3Cscript%3Ealert(1)%3C/script%3E&metric=../x&refresh=1ms");
    const stat = vi.fn<Client["stats"]>().mockResolvedValue(stats(group("fast")));
    const { container } = show(client({ stats: stat }));
    await screen.findByRole("table");
    expect(stat.mock.calls[0]?.[0]).toMatchObject({ window: "24h", groupBy: "target" });
    expect(screen.getByLabelText<HTMLSelectElement>("Metric").value).toBe("failure_rate");
    expect(screen.getByLabelText<HTMLSelectElement>("Refresh").value).toBe("1m");
    expect(container.querySelector("script")).toBeNull();
  });
});

describe("the chart and the table together", () => {
  it("filters the table to a series when it is chosen in the legend, and shows all again when chosen again", async () => {
    const two = stats(group("a"), group("b"));
    show(client({ stats: () => Promise.resolve(two), series: () => Promise.resolve(series({ a: [bucket(0), bucket(1)], b: [bucket(0), bucket(1)] })), health: () => Promise.resolve(health()) }));
    await screen.findByRole("table");
    expect(screen.getAllByRole("rowheader")).toHaveLength(2);
    fireEvent.click(screen.getByRole("button", { name: "b" }));
    expect(screen.getAllByRole("rowheader").map((c) => c.textContent)).toEqual(["b"]);
    expect(window.location.hash).toContain("focus=b");
    fireEvent.click(screen.getByRole("button", { name: "b" }));
    expect(screen.getAllByRole("rowheader")).toHaveLength(2);
    expect(window.location.hash).not.toContain("focus");
  });

  it("shows the ranking with the table's numbers, and says which rest on few calls", async () => {
    const two = stats(group("a", { requests: 10, attempts: 10, failures: 4, failure_rate: 0.4 }), group("b", { requests: 500, attempts: 500, failures: 5, failure_rate: 0.01 }));
    const { container } = show(client({ stats: () => Promise.resolve(two), health: () => Promise.resolve(health()) }));
    await screen.findByRole("table");
    const table = screen.getAllByRole("rowheader").map((c) => c.closest("tr")).map((r) => (r as HTMLElement).querySelectorAll("td")[3]?.textContent);
    fireEvent.change(screen.getByLabelText("Show"), { target: { value: "bars" } });
    const bars = [...container.querySelectorAll(".bar-value")].map((t) => t.textContent);
    expect(bars).toHaveLength(table.length);
    bars.forEach((text, i) => {
      expect(text.startsWith(table[i] ?? "?")).toBe(true);
    });
    expect(bars[0]).toBe("40% · 10 calls");
    expect(screen.getAllByRole("rowheader").map((c) => c.textContent)).toEqual(["a", "b"]);
  });

  it("says the gateway cannot draw a series, keeps the table, and still shows the version warning's place", async () => {
    show(client({ series: () => Promise.resolve(null) }));
    expect((await screen.findByText(/cannot draw a series/)).textContent).toContain("older than this page");
    expect(screen.getByRole("table")).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("leaves a gap and says there were no calls where a group had none", async () => {
    show(client({ series: () => Promise.resolve(series({ fast: [bucket(0), quietBucket(1), bucket(2), bucket(3)] })) }));
    await screen.findByRole("table");
    expect(document.body.textContent).toContain("fast: no calls");
  });
});

describe("heavy reads", () => {
  it("asks for the series and the statistics one after the other, never together", async () => {
    // The gateway allows one heavy read at a time on this surface and refuses a second as busy.
    let heavy = 0;
    let overlapped = false;
    const slow = <T,>(value: T) => {
      heavy += 1;
      if (heavy > 1) overlapped = true;
      return new Promise<T>((resolve) => {
        setTimeout(() => {
          heavy -= 1;
          resolve(value);
        }, 5);
      });
    };
    show(client({ stats: () => slow(stats(group("fast"))), series: () => slow(series({ fast: [bucket(0), bucket(1)] })) }));
    await screen.findByRole("table");
    expect(overlapped).toBe(false);
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("failures and refreshing", () => {
  it("shows the error and no table when the first read fails", async () => {
    show(client({ stats: () => Promise.reject(new ApiError(0, null, "The admin API could not be reached.")) }));
    expect((await screen.findByRole("alert")).textContent).toBe("The admin API could not be reached.");
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("keeps earlier numbers after a failed refresh, marked out of date with the time they are from", async () => {
    const stat = vi
      .fn<Client["stats"]>()
      .mockResolvedValueOnce(stats(group("fast")))
      .mockRejectedValue(new ApiError(0, null, "The admin API could not be reached."));
    const { container } = show(client({ stats: stat }));
    await screen.findByRole("table");
    fireEvent.click(screen.getByText("Refresh now"));
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toMatch(/could not be reached\. The numbers below are out of date: they are from \d\d:\d\d:\d\d\./);
    expect(container.querySelector(".stale")).not.toBeNull();
    expect(screen.getByRole("table")).toBeTruthy();
  });

  it("says a busy gateway is busy, not that the credential failed", async () => {
    const stat = vi
      .fn<Client["stats"]>()
      .mockResolvedValueOnce(stats(group("fast")))
      .mockRejectedValue(new ApiError(503, "admin_busy", "busy", 5000));
    show(client({ stats: stat }));
    await screen.findByRole("table");
    fireEvent.click(screen.getByText("Refresh now"));
    expect((await screen.findByRole("alert")).textContent).toContain("The gateway is busy");
  });

  it("hands back to the sign-in checks when the credential is refused", async () => {
    const onRefused = vi.fn();
    show(client({ stats: () => Promise.reject(new ApiError(401, "admin_credential_invalid", "refused")) }), onRefused);
    await waitFor(() => { expect(onRefused).toHaveBeenCalledTimes(1); });
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("warns when the statistics were cut short", async () => {
    show(client({ stats: () => Promise.resolve({ ...stats(group("fast")), truncated: true }) }));
    expect((await screen.findByText(/cut short/)).textContent).toContain("incomplete");
  });

  it("shows when its numbers are from, and that it is not refreshing when the period is off", async () => {
    window.history.replaceState(null, "", "#refresh=off");
    show(client());
    expect((await screen.findByText(/Updated \d\d:\d\d:\d\d/)).textContent).toContain("not refreshing");
  });
});

describe("the time zone choice", () => {
  afterEach(() => {
    vi.unstubAllEnvs();
  });

  /** The hover texts of the chart's points. */
  const titles = (container: Element) => [...container.querySelectorAll("svg[role='img'] title")].map((t) => t.textContent);

  it("starts in the browser's zone, names it, and shows UTC when asked, from the data already read", async () => {
    vi.stubEnv("TZ", "America/Chicago");
    const ser = vi.fn<Client["series"]>().mockResolvedValue(series({ fast: [bucket(0), bucket(1), bucket(2)] }));
    const stat = vi.fn<Client["stats"]>().mockResolvedValue(stats(group("fast")));
    const { container } = show(client({ series: ser, stats: stat }));
    await screen.findByRole("table");
    expect(screen.getByLabelText<HTMLSelectElement>("Times").value).toBe("local");
    expect(container.textContent).toContain("times are America/Chicago");
    // The fixtures start at 12:00 UTC on 12 November, which is 06:00 in US Central standard time.
    expect(titles(container)[0]).toContain("06:00 CST");

    fireEvent.change(screen.getByLabelText("Times"), { target: { value: "utc" } });
    expect(container.textContent).toContain("times are UTC");
    expect(titles(container)[0]).toContain("12:00 UTC");
    expect(window.location.hash).toContain("zone=utc");
    // The same numbers, written differently: nothing was asked of the gateway again.
    expect(ser).toHaveBeenCalledTimes(1);
    expect(stat).toHaveBeenCalledTimes(1);

    fireEvent.change(screen.getByLabelText("Times"), { target: { value: "local" } });
    expect(titles(container)[0]).toContain("06:00 CST");
    expect(window.location.hash).not.toContain("zone");
    expect(ser).toHaveBeenCalledTimes(1);
  });

  it("opens a link with zone=utc in UTC although the browser's zone is another", async () => {
    vi.stubEnv("TZ", "Asia/Kolkata");
    window.history.replaceState(null, "", "#zone=utc");
    const { container } = show(client());
    await screen.findByRole("table");
    expect(screen.getByLabelText<HTMLSelectElement>("Times").value).toBe("utc");
    expect(container.textContent).toContain("times are UTC");
    expect(titles(container)[0]).toContain("12:00 UTC");
  });

  it("ignores a hostile zone in the fragment", async () => {
    window.history.replaceState(null, "", "#zone=%3Cscript%3E");
    const { container } = show(client());
    await screen.findByRole("table");
    expect(screen.getByLabelText<HTMLSelectElement>("Times").value).toBe("local");
    expect(container.querySelector("script")).toBeNull();
  });
});

describe("the ranking beside the table", () => {
  it("orders the bars by the metric while the table keeps its problems-first order", async () => {
    window.history.replaceState(null, "", "#metric=requests");
    const two = stats(
      group("busy", { requests: 900, attempts: 900, failures: 9, failure_rate: 0.01 }),
      group("failing", { requests: 30, attempts: 30, failures: 20, failure_rate: 0.67 }),
    );
    const { container } = show(client({ stats: () => Promise.resolve(two), health: () => Promise.resolve(health()), series: () => Promise.resolve(series({ busy: [bucket(0)], failing: [bucket(0)] })) }));
    await screen.findByRole("table");
    fireEvent.change(screen.getByLabelText("Show"), { target: { value: "bars" } });
    const bars = [...container.querySelectorAll("g.bar .bar-name")].map((t) => t.textContent);
    const rowsInTable = [...container.querySelectorAll("tbody tr")].map((r) => r.querySelector("th")?.textContent ?? "").filter((t) => t === "busy" || t === "failing");
    expect(bars).toEqual(["busy", "failing"]);
    expect(rowsInTable).toEqual(["failing", "busy"]);
  });
});
