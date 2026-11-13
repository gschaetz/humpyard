import { METRIC_LABELS, formatMetric } from "./chart-data";
import { metricOf, type ModelRow } from "./models";
import type { Metric } from "./query";

const LABEL = 150;
const VALUE = 140;

/** A group with fewer calls than this in the window has its call count printed beside its bar. */
export const THIN_CALLS = 20;
const WIDTH = 640;
const ROW = 26;

function calls(row: ModelRow): number {
  return row.stats?.requests ?? 0;
}

/** Whether a bar rests on so few calls that its count is worth printing. Under the requests metric the value is the count. */
function thin(row: ModelRow, metric: Metric): boolean {
  return metric !== "requests" && metricOf(row, metric) !== null && calls(row) < THIN_CALLS;
}

/**
 * The rows in ranking order: the chosen metric's value, highest first, groups with no value last, ties by name. This is
 * not the table's order (problems first), which answers a different question.
 */
export function ranked(rows: ModelRow[], metric: Metric): ModelRow[] {
  return rows
    .map((row) => ({ row, value: metricOf(row, metric) }))
    .sort((a, b) => {
      if (a.value === null || b.value === null) return Number(a.value === null) - Number(b.value === null) || a.row.group.localeCompare(b.row.group);
      return b.value - a.value || a.row.group.localeCompare(b.row.group);
    })
    .map((entry) => entry.row);
}

/**
 * The chosen metric as a ranking: one bar per group, longest first, from the window's statistics (not the buckets).
 * A group with no value for the metric has no bar and says "no value", not 0. A bar that stands on few calls says how
 * many, so a rate from one call is not taken for a trend. As in the chart, widths are attributes and colours are
 * classes; names are text.
 */
export function Bars(props: { rows: ModelRow[]; metric: Metric; focus: string; onFocus: (group: string) => void }) {
  const { metric, focus } = props;
  const rows = ranked(props.rows, metric);
  const values = rows.map((row) => metricOf(row, metric));
  const max = Math.max(0, ...values.map((v) => v ?? 0));
  const room = WIDTH - LABEL - VALUE;
  return (
    <figure class="chart bars">
      <svg viewBox={`0 0 ${WIDTH.toString()} ${(rows.length * ROW + 8).toString()}`} role="img" aria-label={`${METRIC_LABELS[metric]} by group`}>
        {rows.map((row, i) => {
          const value = values[i] ?? null;
          const y = 4 + i * ROW;
          const chosen = focus === row.group;
          return (
            <g
              key={row.group}
              class={`bar${chosen ? " chosen" : ""}${focus !== "" && !chosen ? " dim" : ""}`}
              role="button"
              tabIndex={0}
              aria-pressed={chosen}
              onClick={() => { props.onFocus(chosen ? "" : row.group); }}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") props.onFocus(chosen ? "" : row.group);
              }}
            >
              <title>{row.group}</title>
              <text class="axis bar-name" x={LABEL - 8} y={y + 15} text-anchor="end">
                {row.group.length > 22 ? `${row.group.slice(0, 21)}…` : row.group}
              </text>
              {value !== null && max > 0 && <rect class="bar-fill" x={LABEL} y={y + 3} width={(value / max) * room} height={ROW - 10} />}
              {value !== null && max === 0 && <rect class="bar-fill" x={LABEL} y={y + 3} width={1} height={ROW - 10} />}
              <text class="axis bar-value" x={LABEL + (value !== null && max > 0 ? (value / max) * room : 0) + 6} y={y + 15}>
                {formatMetric(metric, value)}
                {thin(row, metric) && <tspan class="bar-thin">{` · ${calls(row).toString()} ${calls(row) === 1 ? "call" : "calls"}`}</tspan>}
              </text>
            </g>
          );
        })}
      </svg>
    </figure>
  );
}
