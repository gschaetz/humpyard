import { describe, expect, it } from "vitest";
import { contractNotice, SUPPORTED_CONTRACT_VERSIONS } from "./version";

describe("the contract version check", () => {
  it("says nothing for a version the build was written for", () => {
    expect(contractNotice(SUPPORTED_CONTRACT_VERSIONS[0] ?? "")).toBeNull();
  });

  it("names both versions when the gateway's is not one the build knows", () => {
    const notice = contractNotice("2", ["1"]);
    expect(notice).toContain("version 2");
    expect(notice).toContain("built for 1");
  });
});
