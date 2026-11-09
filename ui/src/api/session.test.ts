import { afterEach, describe, expect, it, vi } from "vitest";
import { clearCredential, loadCredential, saveCredential } from "./session";

afterEach(() => {
  clearCredential();
  vi.restoreAllMocks();
});

describe("the credential's lifetime", () => {
  it("is kept in sessionStorage and never in localStorage or a cookie", () => {
    saveCredential("secret");
    expect(sessionStorage.length).toBe(1);
    expect(localStorage.length).toBe(0);
    expect(document.cookie).toBe("");
    expect(loadCredential()).toBe("secret");
  });

  it("is gone after sign-out", () => {
    saveCredential("secret");
    clearCredential();
    expect(loadCredential()).toBeNull();
    expect(sessionStorage.length).toBe(0);
  });

  it("falls back to memory where storage is blocked", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    saveCredential("secret");
    expect(loadCredential()).toBe("secret");
  });
});
