import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/preact";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiError, type Client } from "./api/client";
import { group, health, stats } from "./fixtures";
import { ModelsView } from "./models-view";

function client(over: Partial<Client> = {}): Client {
  return {
    whoami: () => Promise.reject(new Error("not used")),
    contractVersion: () => Promise.reject(new Error("not used")),
    stats: () => Promise.resolve(stats(group("fast"))),
    series: () => Promise.resolve(null),
    health: () => Promise.resolve(health({ name: "fast", states: ["healthy"], latency: 350 })),
    exchangeToken: () => Promise.reject(new Error("not used")),
    ...over,
  };
}

afterEach(cleanup);

describe("the models view", () => {
  it("shows each target's figures", async () => {
    render(<ModelsView client={client()} />);
    const row = (await screen.findByText("fast")).closest("tr");
    expect(row).not.toBeNull();
    const cells = within(row as HTMLElement).getAllByRole("cell").map((c) => c.textContent);
    expect(cells).toContain("healthy · 350 ms now");
    expect(cells).toContain("25%"); // empty output: 2 of the 8 observed calls
    expect(cells).toContain("1.0k / 500");
    expect(cells).toContain("900 ms");
    expect(cells).toContain("$0.0020");
  });

  it("renders a hostile model name as text and creates no element from it", async () => {
    const hostile = "<img src=x onerror=alert(1)>";
    const { container } = render(
      <ModelsView
        client={client({
          stats: () => Promise.resolve(stats(group(hostile))),
          health: () => Promise.resolve(health({ name: hostile, states: ["healthy"] })),
        })}
      />,
    );
    expect(await screen.findByText(hostile)).toBeTruthy();
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelectorAll("[onerror]")).toHaveLength(0);
  });

  it("shows no signal rate, not 0%, for a target with no observed calls", async () => {
    const unobserved = group("quiet", {
      response_signals: { observed: 0, empty_output: 0, truncated_output: 0, bad_tool_call: 0, cut_stream: 0 },
    });
    render(<ModelsView client={client({ stats: () => Promise.resolve(stats(unobserved)) })} />);
    const row = (await screen.findByText("quiet")).closest("tr") as HTMLElement;
    const cells = within(row).getAllByRole("cell").map((c) => c.textContent);
    expect(cells.slice(5, 9)).toEqual(["—", "—", "—", "—"]);
    expect(within(row).queryByText("0%")).toBeNull();
  });

  it("lists a configured target with no calls without inventing figures", async () => {
    render(
      <ModelsView
        client={client({
          stats: () => Promise.resolve(stats()),
          health: () => Promise.resolve(health({ name: "idle", states: ["healthy"] })),
        })}
      />,
    );
    const row = (await screen.findByText("idle")).closest("tr") as HTMLElement;
    const cells = within(row).getAllByRole("cell").map((c) => c.textContent);
    expect(cells[1]).toBe("0");
    expect(cells.slice(3, 9)).toEqual(["—", "—", "—", "—", "—", "—"]);
  });

  it("asks for the chosen window", async () => {
    const stat = vi.fn<Client["stats"]>().mockResolvedValue(stats(group("fast")));
    render(<ModelsView client={client({ stats: stat })} />);
    await screen.findByText("fast");
    expect(stat).toHaveBeenLastCalledWith({ window: "24h", groupBy: "target" });
    fireEvent.change(screen.getByLabelText("Window"), { target: { value: "7d" } });
    await waitFor(() => { expect(stat).toHaveBeenLastCalledWith({ window: "7d", groupBy: "target" }); });
  });

  it("shows the error, and not the last numbers, when the API cannot be reached", async () => {
    const stat = vi
      .fn<Client["stats"]>()
      .mockResolvedValueOnce(stats(group("fast")))
      .mockRejectedValue(new ApiError(0, null, "The admin API could not be reached."));
    render(<ModelsView client={client({ stats: stat })} />);
    await screen.findByText("fast");
    fireEvent.click(screen.getByText("Refresh"));
    expect((await screen.findByRole("alert")).textContent).toBe("The admin API could not be reached.");
    expect(screen.queryByText("fast")).toBeNull();
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("warns when the statistics were cut short", async () => {
    const cut = { ...stats(group("fast")), truncated: true };
    render(<ModelsView client={client({ stats: () => Promise.resolve(cut) })} />);
    expect((await screen.findByRole("status")).textContent).toContain("cut short");
  });
});
