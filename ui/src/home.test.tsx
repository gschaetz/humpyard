import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/preact";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./app";
import { ApiError, type Client } from "./api/client";
import type { ConnectInfo, KeyStatus, LocalClient, TestResult } from "./api/local";
import { bucket, config, group, health, series, stats, withBaseline } from "./fixtures";
import { UpHome } from "./home";

const SECRET = "gsk-this-must-never-appear-on-the-page";
const info = (over: Partial<ConnectInfo> = {}): ConnectInfo => ({
  gateway: "http://127.0.0.1:8080",
  routes: ["deep", "fast"],
  has_config: true,
  gateway_running: true,
  key_entry: true,
  ...over,
});
const groq = (source: KeyStatus["source"] = "none"): KeyStatus => ({ provider: "groq", variable: "GROQ_API_KEY", source });

function local(over: Partial<LocalClient> = {}): LocalClient {
  return {
    connect: () => Promise.resolve(info()),
    keys: () => Promise.resolve({ key_entry: true, providers: [groq()] }),
    setKey: () => Promise.resolve("held for this run only; the gateway restarted"),
    removeKey: () => Promise.resolve("removed"),
    test: () => Promise.resolve({ ok: true, model: "fast", target: "groq/fast-m", latency_ms: 312 }),
    ...over,
  };
}

function client(over: Partial<Client> = {}): Client {
  return {
    whoami: () => Promise.resolve({ id: "up", scopes: ["read"] } as Awaited<ReturnType<Client["whoami"]>>),
    contractVersion: () => Promise.resolve("1"),
    stats: () => Promise.resolve(stats(group("fast", { answered: 0, requests: 0 }))),
    series: () => Promise.resolve(series({ fast: [bucket(0)] })),
    config: () => Promise.resolve(config({ name: "p/big", price: { input: 3, output: 15 } })),
    health: () => Promise.resolve(health({ name: "fast", states: ["healthy"], latency: 300 })),
    exchangeToken: () => Promise.reject(new Error("not used")),
    ...over,
  };
}

function home(c: Client, l: LocalClient, i: ConnectInfo = info()) {
  return render(<UpHome client={c} local={l} info={i} onRefused={() => undefined} onChanged={() => undefined} />);
}

beforeEach(() => {
  window.history.replaceState(null, "", "#");
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("a gateway that has answered nothing", () => {
  it("says so, offers the connect settings and a test request, and shows no empty chart", async () => {
    home(client(), local());
    expect(await screen.findByText("Nothing has gone through the gateway yet")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Send a test request" })).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Connect a tool" })).toBeTruthy();
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("shows which target answered the test request and how fast, and moves on to the query view", async () => {
    home(client(), local());
    fireEvent.click(await screen.findByRole("button", { name: "Send a test request" }));
    const result = await screen.findByText(/The test request worked/);
    expect(result.textContent).toContain("groq/fast-m");
    expect(result.textContent).toContain("312 ms");
    expect(screen.queryByText("Nothing has gone through the gateway yet")).toBeNull();
    expect(await screen.findByRole("table")).toBeTruthy();
  });

  it("says which step failed, in words", async () => {
    const test = (): Promise<TestResult> => Promise.resolve({ ok: false, step: "provider", message: "The provider turned the request away. Check its key." });
    home(client(), local({ test }));
    fireEvent.click(await screen.findByRole("button", { name: "Send a test request" }));
    expect((await screen.findByRole("alert")).textContent).toBe("The provider turned the request away. Check its key.");
  });
});

describe("the connect panel", () => {
  it("shows the live address and a placeholder, and copies exactly what it shows", async () => {
    const writeText = vi.fn<(text: string) => Promise<void>>().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    home(client(), local());
    const shown = (await screen.findByText(/ANTHROPIC_BASE_URL/)).textContent;
    expect(shown).toContain("ANTHROPIC_BASE_URL=http://127.0.0.1:8080");
    expect(shown).toContain("ANTHROPIC_API_KEY=humpyard");
    fireEvent.click(screen.getByRole("button", { name: "Copy" }));
    await waitFor(() => { expect(writeText).toHaveBeenCalledWith(shown); });
    expect((await screen.findByText("Copied.")).textContent).toBe("Copied.");
  });

  it("switches to another tool's settings", async () => {
    home(client(), local());
    fireEvent.click(await screen.findByRole("tab", { name: "Codex CLI" }));
    expect((await screen.findByText(/model_providers\.humpyard/)).textContent).toContain('base_url = "http://127.0.0.1:8080/v1"');
  });
});

describe("with no gateway running", () => {
  const down = info({ gateway_running: false });

  it("asks for a key before anything else", async () => {
    home(client(), local(), down);
    expect(await screen.findByText("Add a key to start the gateway")).toBeTruthy();
    expect(await screen.findByLabelText("Key for groq")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Send a test request" })).toBeNull();
  });

  it("holds a key for this run unless asked to save it, and says what the choices are", async () => {
    const setKey = vi.fn<LocalClient["setKey"]>().mockResolvedValue("held for this run only; the gateway restarted");
    home(client(), local({ setKey }), down);
    const field = await screen.findByLabelText("Key for groq");
    expect(screen.getByLabelText<HTMLInputElement>("Use for this run only").checked).toBe(true);
    expect(screen.getByLabelText<HTMLInputElement>("Save on this machine").checked).toBe(false);
    fireEvent.input(field, { target: { value: SECRET } });
    fireEvent.click(screen.getByRole("button", { name: "Use this key" }));
    await waitFor(() => { expect(setKey).toHaveBeenCalledWith("groq", SECRET, false); });
    expect(await screen.findByText("held for this run only; the gateway restarted")).toBeTruthy();
  });

  it("saves only when that choice is made", async () => {
    const setKey = vi.fn<LocalClient["setKey"]>().mockResolvedValue("saved on this machine; the gateway restarted");
    home(client(), local({ setKey }), down);
    fireEvent.click(await screen.findByLabelText("Save on this machine"));
    fireEvent.input(screen.getByLabelText("Key for groq"), { target: { value: SECRET } });
    fireEvent.click(screen.getByRole("button", { name: "Use this key" }));
    await waitFor(() => { expect(setKey).toHaveBeenCalledWith("groq", SECRET, true); });
  });

  it("empties the field at once, and never shows the key, even when it is refused", async () => {
    const setKey = () => Promise.reject(new ApiError(422, "provider_rejected_key", "groq did not accept this key: answered 401"));
    const { container } = home(client(), local({ setKey }), down);
    const field = await screen.findByLabelText<HTMLInputElement>("Key for groq");
    fireEvent.input(field, { target: { value: SECRET } });
    fireEvent.click(screen.getByRole("button", { name: "Use this key" }));
    expect((await screen.findByRole("alert")).textContent).toBe("groq did not accept this key: answered 401");
    expect(field.value).toBe("");
    expect(container.textContent).not.toContain(SECRET);
    expect(sessionStorage.length).toBe(0);
    expect(localStorage.length).toBe(0);
    expect(window.location.href).not.toContain(SECRET);
  });

  it("shows where each key comes from, and removes one the person put there", async () => {
    const removeKey = vi.fn<LocalClient["removeKey"]>().mockResolvedValue("removed");
    const keys = () => Promise.resolve({ key_entry: true, providers: [groq("file")] });
    home(client(), local({ keys, removeKey }), down);
    expect(await screen.findByText("has a key saved on this machine")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    await waitFor(() => { expect(removeKey).toHaveBeenCalledWith("groq"); });
  });

  it("offers no remove for a key that comes from the environment", async () => {
    const keys = () => Promise.resolve({ key_entry: true, providers: [groq("environment")] });
    home(client(), local({ keys }), down);
    expect(await screen.findByText("has a key from your environment")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Remove" })).toBeNull();
  });

  it("has no key form, and says why, when the page is served beyond this machine", async () => {
    const keys = () => Promise.resolve({ key_entry: false, providers: [groq()] });
    home(client(), local({ keys }), info({ gateway_running: false, key_entry: false }));
    expect(await screen.findByText(/served beyond this machine/)).toBeTruthy();
    expect(screen.queryByLabelText("Key for groq")).toBeNull();
  });
});

describe("once a call has been answered", () => {
  const traffic = () => Promise.resolve(stats(group("fast")));

  it("shows the query view with the savings comparison chosen, and the connect settings folded away", async () => {
    const stat = vi.fn<Client["stats"]>().mockImplementation((query) =>
      Promise.resolve(query?.baseline === undefined ? stats(group("fast")) : withBaseline(stats(group("fast")), { endpoint: "p/big" })),
    );
    home(client({ stats: stat }), local());
    expect(await screen.findByLabelText("What this traffic would have cost")).toBeTruthy();
    expect(screen.queryByText("Nothing has gone through the gateway yet")).toBeNull();
    expect(window.location.hash).toContain("baseline=p%2Fbig");
    expect(screen.getByText("Connect a tool", { selector: "summary" })).toBeTruthy();
  });

  it("lets the person choose no comparison and keeps that choice", async () => {
    const stat = vi.fn<Client["stats"]>().mockImplementation((query) =>
      Promise.resolve(query?.baseline === undefined ? stats(group("fast")) : withBaseline(stats(group("fast")), { endpoint: "p/big" })),
    );
    home(client({ stats: stat }), local());
    await screen.findByLabelText("What this traffic would have cost");
    fireEvent.change(screen.getByLabelText("Compare with"), { target: { value: "" } });
    await waitFor(() => { expect(screen.queryByLabelText("What this traffic would have cost")).toBeNull(); });
    expect(window.location.hash).not.toContain("baseline=");
  });

  it("shows no strip when there is nothing priced to compare with", async () => {
    home(client({ stats: traffic, config: () => Promise.resolve(config({ name: "p/free", price: null })) }), local());
    await screen.findByRole("table");
    expect(screen.queryByLabelText("What this traffic would have cost")).toBeNull();
  });
});

describe("the shell", () => {
  it("opens the setup screen when humpyard up is serving and no gateway is answering", async () => {
    const whoami = () => Promise.reject(new ApiError(502, "upstream_unreachable", "The gateway could not be reached."));
    const l = local({ connect: () => Promise.resolve(info({ gateway_running: false })) });
    render(<App client={client({ whoami })} local={l} />);
    expect(await screen.findByText("Add a key to start the gateway")).toBeTruthy();
  });

  it("is the page it was under humpyard ui: one probe, no first-run screens", async () => {
    const connect = vi.fn<LocalClient["connect"]>().mockResolvedValue(null);
    render(<App client={client()} local={local({ connect })} />);
    expect(await screen.findByText("up")).toBeTruthy();
    expect(connect).toHaveBeenCalledTimes(1);
    expect(screen.queryByText("Connect a tool")).toBeNull();
    expect(screen.queryByLabelText("Key for groq")).toBeNull();
  });

  it("still reports an unreachable admin API when nothing under /local answers either", async () => {
    const whoami = () => Promise.reject(new ApiError(0, null, "The admin API could not be reached."));
    render(<App client={client({ whoami })} local={local({ connect: () => Promise.resolve(null) })} />);
    expect((await screen.findByRole("alert")).textContent).toBe("The admin API could not be reached.");
  });

  it("opens the home screen of humpyard up when the gateway is running", async () => {
    render(<App client={client()} local={local()} />);
    expect(await screen.findByText("Nothing has gone through the gateway yet")).toBeTruthy();
    expect(within(document.body).getByRole("heading", { name: "Connect a tool" })).toBeTruthy();
  });
});
