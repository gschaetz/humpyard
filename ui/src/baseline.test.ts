import { describe, expect, it } from "vitest";
import { ApiError } from "./api/client";
import { describeBaseline, effectiveBaseline, isUnknownBaseline, money, offeredBaselines, perMillion } from "./baseline";
import { config, withBaseline } from "./fixtures";
import { group, stats } from "./fixtures";

describe("the endpoints a baseline can be", () => {
  it("are the priced endpoints without tiers, dearest output price first, each once", () => {
    const offered = offeredBaselines(
      config(
        { name: "free/m", price: { input: 0, output: 0 } },
        { name: "openai/big", price: { input: 3, output: 15 } },
        { name: "groq/llama", price: null },
        { name: "anthropic/tiered", price: { input: 3, output: 15, tiers: [{ above_tokens: 100000 }] } },
        { name: "openai/small", price: { input: 0.15, output: 0.6 } },
        { name: "openai/big", price: { input: 9, output: 9 } },
        { name: "meta/x/y", price: { input: 1, output: 1 } },
      ),
    );
    expect(offered.map((o) => o.name)).toEqual(["openai/big", "meta/x/y", "openai/small", "free/m"]);
    // The first target that names an endpoint wins.
    expect(offered[0]).toEqual({ name: "openai/big", input: 3, output: 15 });
  });

  it("are none when nothing is priced or the configuration has no targets", () => {
    expect(offeredBaselines(config({ name: "a/b", price: null }))).toEqual([]);
    expect(offeredBaselines({ targets: {} })).toEqual([]);
  });

  it("are the only values the fragment can select", () => {
    const offered = offeredBaselines(config({ name: "openai/big", price: { input: 3, output: 15 } }));
    expect(effectiveBaseline("openai/big", offered)).toBe("openai/big");
    for (const wanted of ["", "openai", "OPENAI/big", "<script>alert(1)</script>", "openai/big ", "other/x"]) {
      expect(effectiveBaseline(wanted, offered)).toBe("");
    }
    expect(effectiveBaseline("openai/big", [])).toBe("");
  });
});

describe("what the strip says", () => {
  const base = withBaseline(stats(group("a")));
  const text = (over = {}) => {
    const answer = withBaseline(stats(group("a")), over);
    return describeBaseline(answer.summary, answer.baseline);
  };

  it("names the endpoint, its prices, both costs and the saving in words", () => {
    const { headline } = text();
    expect(headline).toBe("At p/big ($3 in, $15 out per million) this traffic would have cost $0.018. It cost $0.012: $0.006 less (33%).");
  });

  it("says more, and not a negative saving, when the baseline would have been cheaper", () => {
    const { headline } = text({ summary: { baseline_cost_usd: 0.01, cost_usd: 0.03, saved_usd: -0.02, saved_ratio: -2 } });
    expect(headline).toContain("$0.02 more than that (200%)");
    expect(headline).not.toContain("-");
  });

  it("says the same when nothing was saved, and has no share when the baseline cost nothing", () => {
    expect(text({ summary: { saved_usd: 0, saved_ratio: 0 } }).headline).toContain(": the same.");
    expect(text({ summary: { baseline_cost_usd: 0, cost_usd: 0, saved_usd: 0, saved_ratio: null } }).headline).not.toContain("%");
  });

  it("says there is nothing to price when no call had usage", () => {
    const { headline } = text({ summary: { calls_priced: 0, baseline_cost_usd: 0, cost_usd: 0, saved_usd: 0, saved_ratio: null } });
    expect(headline).toBe("No answered calls with known usage in this window, so there is nothing to price at p/big.");
  });

  it("counts the calls left out and always carries the statement of what the figure is", () => {
    expect(text().notes).toHaveLength(1);
    expect(text().notes[0]).toContain("estimate");
    const notes = text({ summary: { calls_left_out: 3 } }).notes;
    expect(notes[0]).toBe("3 answered calls reported no usage and are in neither figure.");
    expect(text({ summary: { calls_left_out: 1 } }).notes[0]).toBe("1 answered call reported no usage and is in neither figure.");
    expect(base.baseline.basis).toContain("estimate");
  });

  it("writes dollars for a sentence with as few digits as tell the figure apart", () => {
    expect([money(1.84), money(1234.5), money(0.07), money(0.018), money(0.006), money(0.0004), money(0), money(1)]).toEqual([
      "$1.84", "$1234.50", "$0.07", "$0.018", "$0.006", "$0.0004", "$0", "$1.00",
    ]);
  });

  it("writes a price per million without trailing zeros", () => {
    expect([perMillion(3), perMillion(0.15), perMillion(0), perMillion(1.25), perMillion(0.00006)]).toEqual(["$3", "$0.15", "$0", "$1.25", "$0.0001"]);
  });
});

describe("a gateway that does not know baseline", () => {
  it("is recognised by the 400 that names the parameter and by nothing else", () => {
    expect(isUnknownBaseline(new ApiError(400, null, "unknown parameter `baseline`"))).toBe(true);
    expect(isUnknownBaseline(new ApiError(400, null, "`baseline` `p/x` has no price in the configuration"))).toBe(false);
    expect(isUnknownBaseline(new ApiError(500, null, "unknown parameter `baseline`"))).toBe(false);
    expect(isUnknownBaseline(new Error("unknown parameter `baseline`"))).toBe(false);
  });
});
