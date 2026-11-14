import type { Baseline, BaselineComparison, Config } from "./api/client";
import { percent } from "./format";

/** An endpoint the person may compare the traffic with: priced, and without tiers (`add-baseline-cost`). */
export interface Offered {
  /** `provider/model`, as the statistics name an endpoint. */
  name: string;
  input: number;
  output: number;
}

/**
 * The endpoints a baseline can be, from the effective configuration: every endpoint of every target that has a price
 * with no tiers, the first target that names an endpoint winning, the dearest output price first (a baseline is usually
 * the model you would otherwise have used for everything).
 */
export function offeredBaselines(config: Config): Offered[] {
  const seen = new Map<string, Offered>();
  for (const target of Object.values(config.targets)) {
    for (const endpoint of target.endpoints) {
      const name = `${endpoint.provider}/${endpoint.model}`;
      const price = endpoint.price;
      if (seen.has(name) || price === null || price === undefined) continue;
      if ((price.tiers ?? []).length > 0) continue;
      seen.set(name, { name, input: price.input, output: price.output });
    }
  }
  return [...seen.values()].sort((a, b) => b.output - a.output || a.name.localeCompare(b.name));
}

/** The baseline the page uses: the one the fragment names when it is exactly an offered endpoint, otherwise none. */
export function effectiveBaseline(wanted: string, offered: Offered[]): string {
  return offered.find((o) => o.name === wanted)?.name ?? "";
}

/** Dollars for a sentence: two decimals from a dollar up, otherwise as few as tell the figure apart (`$0.07`, `$0.018`, `$0.006`). */
export function money(value: number): string {
  if (value >= 1) return `$${value.toFixed(2)}`;
  if (value >= 0.01) return `$${Number(value.toFixed(3)).toString()}`;
  if (value === 0) return "$0";
  return `$${Number(value.toPrecision(2)).toString()}`;
}

/** A price per million tokens, as `$3` or `$0.15`. */
export function perMillion(value: number): string {
  return `$${Number(value.toFixed(4)).toString()}`;
}

/** Whether a refusal is the gateway saying it does not know `baseline` (it predates the parameter). */
export function isUnknownBaseline(failure: unknown): boolean {
  const f = failure as { status?: unknown; message?: unknown };
  return f.status === 400 && typeof f.message === "string" && f.message.includes("unknown parameter `baseline`");
}

/** What the strip says, as plain text for the person: the sentence, and what the figure is and leaves out. */
export function describeBaseline(summary: BaselineComparison, baseline: Baseline): { headline: string; notes: string[] } {
  const notes: string[] = [];
  if (summary.calls_left_out > 0) {
    const n = summary.calls_left_out;
    notes.push(`${n.toString()} answered ${n === 1 ? "call" : "calls"} reported no usage and ${n === 1 ? "is" : "are"} in neither figure.`);
  }
  notes.push(baseline.basis);
  if (summary.calls_priced === 0) {
    return { headline: `No answered calls with known usage in this window, so there is nothing to price at ${baseline.endpoint}.`, notes };
  }
  const prices = `${perMillion(baseline.input_per_million_usd)} in, ${perMillion(baseline.output_per_million_usd)} out per million`;
  const start = `At ${baseline.endpoint} (${prices}) this traffic would have cost ${money(summary.baseline_cost_usd)}. It cost ${money(summary.cost_usd)}`;
  const share = summary.saved_ratio === null ? "" : ` (${percent(Math.abs(summary.saved_ratio))})`;
  if (summary.saved_usd > 0) return { headline: `${start}: ${money(summary.saved_usd)} less${share}.`, notes };
  if (summary.saved_usd < 0) return { headline: `${start}: ${money(-summary.saved_usd)} more than that${share}.`, notes };
  return { headline: `${start}: the same.`, notes };
}
