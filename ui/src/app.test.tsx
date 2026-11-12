import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/preact";
import { afterEach, describe, expect, it, vi } from "vitest";
import { App } from "./app";
import { ApiError, type Client } from "./api/client";
import { clearCredential } from "./api/session";

const principal = { id: "gary", scopes: ["read"] } as Awaited<ReturnType<Client["whoami"]>>;

function fake(overrides: Partial<Client> = {}): Client {
  return {
    whoami: () => Promise.resolve(principal),
    contractVersion: () => Promise.resolve("1"),
    stats: () => Promise.reject(new Error("not used")),
    series: () => Promise.reject(new Error("not used")),
    health: () => Promise.reject(new Error("not used")),
    exchangeToken: () => Promise.reject(new Error("not used")),
    ...overrides,
  };
}

afterEach(() => {
  cleanup();
  clearCredential();
});

describe("the shell", () => {
  it("goes straight in when the API already accepts us (a credential held by humpyard ui)", async () => {
    render(<App client={fake()} />);
    expect(await screen.findByText("gary")).toBeTruthy();
    expect(screen.queryByText("Sign in")).toBeNull();
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("asks for a credential when the API refuses us", async () => {
    const refuse = () => Promise.reject(new ApiError(401, "admin_credential_invalid", "Invalid admin credential."));
    render(<App client={fake({ whoami: refuse })} />);
    expect(await screen.findByLabelText("Admin credential")).toBeTruthy();
  });

  it("reports a wrong credential in the API's words and keeps nothing", async () => {
    const refuse = () => Promise.reject(new ApiError(401, "admin_credential_invalid", "Invalid admin credential."));
    render(<App client={fake({ whoami: refuse })} />);
    fireEvent.input(await screen.findByLabelText("Admin credential"), { target: { value: "wrong" } });
    fireEvent.click(screen.getByText("Sign in"));
    expect((await screen.findByRole("alert")).textContent).toBe("Invalid admin credential.");
    expect(sessionStorage.length).toBe(0);
  });

  it("keeps an accepted credential for the session and shows the data", async () => {
    const whoami = vi
      .fn<Client["whoami"]>()
      .mockRejectedValueOnce(new ApiError(401, null, "no"))
      .mockResolvedValue(principal);
    render(<App client={fake({ whoami })} />);
    fireEvent.input(await screen.findByLabelText("Admin credential"), { target: { value: "right" } });
    fireEvent.click(screen.getByText("Sign in"));
    expect(await screen.findByText("gary")).toBeTruthy();
    expect(sessionStorage.length).toBe(1);
    expect(localStorage.length).toBe(0);
  });

  it("forgets the credential on sign-out", async () => {
    sessionStorage.setItem("humpyard.admin.credential", "right");
    render(<App client={fake()} />);
    fireEvent.click(await screen.findByText("Sign out"));
    expect(await screen.findByLabelText("Admin credential")).toBeTruthy();
    expect(sessionStorage.length).toBe(0);
  });

  it("warns about a newer gateway, naming both versions, and still shows the data", async () => {
    render(<App client={fake({ contractVersion: () => Promise.resolve("2") })} />);
    const notice = await screen.findByRole("status");
    expect(notice.textContent).toContain("version 2");
    expect(notice.textContent).toContain("built for 1");
    expect(screen.getByText("gary")).toBeTruthy();
  });

  it("warns, and still shows the data, when the version cannot be read", async () => {
    const broken = () => Promise.reject(new ApiError(500, null, "boom"));
    render(<App client={fake({ contractVersion: broken })} />);
    expect((await screen.findByRole("status")).textContent).toContain("could not be read");
    expect(screen.getByText("gary")).toBeTruthy();
  });

  it("says so when the gateway cannot be reached, and tries again on request", async () => {
    const whoami = vi
      .fn<Client["whoami"]>()
      .mockRejectedValueOnce(new ApiError(0, null, "The admin API could not be reached."))
      .mockResolvedValue(principal);
    render(<App client={fake({ whoami })} />);
    expect((await screen.findByRole("alert")).textContent).toBe("The admin API could not be reached.");
    fireEvent.click(screen.getByText("Try again"));
    await waitFor(() => { expect(screen.getByText("gary")).toBeTruthy(); });
  });

  it("asks for humpyard ui's access token when it holds the credential, and goes in once it is exchanged", async () => {
    const whoami = vi
      .fn<Client["whoami"]>()
      .mockRejectedValueOnce(new ApiError(401, "ui_token_required", "Enter the access token."))
      .mockResolvedValue(principal);
    const exchangeToken = vi.fn<Client["exchangeToken"]>().mockResolvedValue(undefined);
    render(<App client={fake({ whoami, exchangeToken })} />);
    fireEvent.input(await screen.findByLabelText("Access token"), { target: { value: "tok" } });
    fireEvent.click(screen.getByText("Continue"));
    expect(await screen.findByText("gary")).toBeTruthy();
    expect(exchangeToken).toHaveBeenCalledWith("tok");
    expect(sessionStorage.length).toBe(0);
  });

  it("reports a wrong access token in the service's words", async () => {
    const need = () => Promise.reject(new ApiError(401, "ui_token_required", "Enter the access token."));
    const exchangeToken = () => Promise.reject(new ApiError(401, "ui_token_invalid", "That is not the access token."));
    render(<App client={fake({ whoami: need, exchangeToken })} />);
    fireEvent.input(await screen.findByLabelText("Access token"), { target: { value: "bad" } });
    fireEvent.click(screen.getByText("Continue"));
    expect((await screen.findByRole("alert")).textContent).toBe("That is not the access token.");
  });
});
