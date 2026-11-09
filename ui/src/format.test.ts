import { describe, expect, it } from "vitest";
import { count, ms, percent, tokens, usd } from "./format";

describe("formatting", () => {
  it("shows a dash, never 0, for a figure that does not exist", () => {
    expect([percent(null), ms(null), usd(null)]).toEqual(["—", "—", "—"]);
  });

  it("shows a real zero as zero", () => {
    expect([percent(0), usd(0), tokens(0), count(0)]).toEqual(["0%", "$0", "0", "0"]);
  });

  it("rounds sensibly", () => {
    expect(percent(0.0166)).toBe("1.7%");
    expect(percent(0.25)).toBe("25%");
    expect(usd(0.002)).toBe("$0.0020");
    expect(usd(1.23456)).toBe("$1.235");
    expect(tokens(1234)).toBe("1.2k");
    expect(tokens(4_500_000)).toBe("4.5M");
    expect(ms(349.6)).toBe("350 ms");
  });
});
