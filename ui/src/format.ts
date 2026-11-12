const DASH = "—";

/** A share as a percentage; the dash (not 0%) when there is nothing to take a share of. */
export function percent(value: number | null): string {
  if (value === null) return DASH;
  const p = value * 100;
  return `${p >= 10 || p === 0 ? p.toFixed(0) : p.toFixed(1)}%`;
}

export function count(value: number): string {
  return new Intl.NumberFormat("en-US").format(value);
}

export function ms(value: number | null): string {
  return value === null ? DASH : `${new Intl.NumberFormat("en-US", { maximumFractionDigits: 0 }).format(value)} ms`;
}

export function usd(value: number | null): string {
  if (value === null) return DASH;
  if (value === 0) return "$0";
  return value < 0.01 ? `$${value.toPrecision(2)}` : `$${value.toFixed(3)}`;
}

/** Tokens as a short figure (12.3k, 4.5M). */
export function tokens(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1)}M`;
  if (value >= 1_000) return `${(value / 1_000).toFixed(1)}k`;
  return value.toString();
}

/** The local time of day as hh:mm:ss. */
export function clock(atMs: number): string {
  const d = new Date(atMs);
  return [d.getHours(), d.getMinutes(), d.getSeconds()].map((n) => n.toString().padStart(2, "0")).join(":");
}
