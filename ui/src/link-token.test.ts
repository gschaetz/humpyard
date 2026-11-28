import { afterEach, describe, expect, it } from "vitest";
import { takeToken, takeTokenFromLocation } from "./link-token";

describe("takeToken", () => {
  it("finds the token and leaves the rest of the fragment", () => {
    expect(takeToken("#token=abc123")).toEqual({ token: "abc123", rest: "" });
    expect(takeToken("#window=7d&token=abc123&by=route")).toEqual({ token: "abc123", rest: "#window=7d&by=route" });
  });

  it("leaves a fragment without a token alone", () => {
    expect(takeToken("#window=7d")).toEqual({ token: null, rest: "#window=7d" });
    expect(takeToken("")).toEqual({ token: null, rest: "" });
  });

  it("treats an empty token as none", () => {
    expect(takeToken("#token=")).toEqual({ token: null, rest: "" });
  });
});

describe("takeTokenFromLocation", () => {
  afterEach(() => {
    window.history.replaceState(null, "", "/");
  });

  it("removes the token from the address and keeps the query", () => {
    window.history.replaceState(null, "", "/#window=7d&token=secret-token");
    expect(takeTokenFromLocation()).toBe("secret-token");
    expect(window.location.hash).toBe("#window=7d");
    expect(window.location.href).not.toContain("secret-token");
  });

  it("does nothing when there is no token", () => {
    window.history.replaceState(null, "", "/#window=7d");
    expect(takeTokenFromLocation()).toBeNull();
    expect(window.location.hash).toBe("#window=7d");
  });
});
