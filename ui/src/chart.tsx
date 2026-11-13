import { useEffect, useRef, useState } from "preact/hooks";
import type { Series } from "./api/client";
import { BOTTOM, HEIGHT, LEFT, MAX_SERIES, METRIC_LABELS, MIN_WIDTH, RIGHT, TOP, WIDTH, formatMetric, layout, pointTitle, strokes, timeText, type Layout } from "./chart-data";
import type { Metric } from "./query";

/** A marker whose shape tells a series apart without colour. `shape` runs 0 to 7. */
export function Marker(props: { shape: number; x: number; y: number }) {
  const { x, y } = props;
  const r = 4;
  const f = (value: number) => value.toFixed(1);
  const poly = (offsets: [number, number][]) => offsets.map(([dx, dy]) => `${f(x + dx)},${f(y + dy)}`).join(" ");
  switch (props.shape % MAX_SERIES) {
    case 0:
      return <circle class="mark" cx={x} cy={y} r={r} />;
    case 1:
      return <rect class="mark" x={x - r} y={y - r} width={2 * r} height={2 * r} />;
    case 2:
      return <polygon class="mark" points={poly([[0, -r - 1], [r + 1, r], [-r - 1, r]])} />;
    case 3:
      return <polygon class="mark" points={poly([[0, -r - 1], [r + 1, 0], [0, r + 1], [-r - 1, 0]])} />;
    case 4:
      return <polygon class="mark" points={poly([[0, r + 1], [r + 1, -r], [-r - 1, -r]])} />;
    case 5:
      return <path class="mark stroke" d={`M${f(x - r)} ${f(y)}H${f(x + r)}M${f(x)} ${f(y - r)}V${f(y + r)}`} />;
    case 6:
      return <path class="mark stroke" d={`M${f(x - r)} ${f(y - r)}L${f(x + r)} ${f(y + r)}M${f(x + r)} ${f(y - r)}L${f(x - r)} ${f(y + r)}`} />;
    default:
      return <polygon class="mark" points={poly([[-r, -r / 2], [0, -r - 1], [r, -r / 2], [r, r / 2], [0, r + 1], [-r, r / 2]])} />;
  }
}

const TIP_ROW = 16;
const TIP_CHAR = 6.6;

function clip(name: string): string {
  return name.length > 24 ? `${name.slice(0, 23)}…` : name;
}

/**
 * What the pointer is over: a vertical line at the nearest bucket, a dot on every series there, and a box listing the
 * time and each series' value and request count. Drawn as SVG (positions are attributes, colours are classes), so the
 * Content-Security-Policy is untouched. Names are text.
 */
function Tooltip(props: { drawn: Layout; index: number; metric: Metric; width: number; timeZone: string }) {
  const { drawn, index, metric, width, timeZone } = props;
  const column = drawn.lines.map((line) => line.points[index]);
  const first = column.find((p) => p !== undefined);
  if (first === undefined) return null;
  const rows = drawn.lines.map((line, i) => {
    const p = line.points[index];
    const what = p === undefined || p.value === null ? "no calls" : `${formatMetric(metric, p.value)} · ${p.requests.toString()} req`;
    return { shape: i, text: `${clip(line.group)}  ${what}` };
  });
  const boxWidth = Math.max(timeText(first.start_ms, timeZone).length, ...rows.map((r) => r.text.length)) * TIP_CHAR + 30;
  const boxHeight = 24 + rows.length * TIP_ROW;
  const left = first.x + 14 + boxWidth > width - 4 ? first.x - 14 - boxWidth : first.x + 14;
  return (
    <g class="tip-layer">
      <line class="cursor" x1={first.x} x2={first.x} y1={TOP} y2={HEIGHT - BOTTOM} />
      {drawn.lines.map((line, i) => {
        const p = line.points[index];
        return p?.y !== undefined && p.y !== null ? (
          <g key={line.group} class={`line s${(i % MAX_SERIES).toString()}`}>
            <circle class="hover-dot" cx={p.x} cy={p.y} r={5} />
          </g>
        ) : null;
      })}
      <g class="tip">
        <rect class="tip-box" x={left} y={TOP + 4} width={boxWidth} height={boxHeight} rx={4} />
        <text class="tip-time" x={left + 8} y={TOP + 20}>{timeText(first.start_ms, timeZone)}</text>
        {rows.map((row, n) => (
          <g key={row.text} class={`line s${(row.shape % MAX_SERIES).toString()}`}>
            <Marker shape={row.shape} x={left + 13} y={TOP + 20 + (n + 1) * TIP_ROW - 4} />
            <text class="tip-row" x={left + 24} y={TOP + 20 + (n + 1) * TIP_ROW}>{row.text}</text>
          </g>
        ))}
      </g>
    </g>
  );
}

/**
 * The metric over time, one line per group, drawn as SVG from the answer. All colours and strokes come from classes
 * in the stylesheet (`s0` to `s7`, `partial`, `dim`) and all geometry from attributes: no `style` attribute is written,
 * so the page's Content-Security-Policy is untouched. Every name in it is text. A bucket with no value is a gap and
 * says so on hover; the last bucket is dashed because it is still filling; a window the row cap cut is shaded and said so.
 * The legend is the filter control: choosing a series dims the others and (in the view) filters the table to it.
 */
export function LineChart(props: {
  series: Series;
  metric: Metric;
  focus: string;
  onFocus: (group: string) => void;
  /** The zone times are written in, as a name Intl knows; UTC when not given. */
  timeZone?: string;
}) {
  const { series, metric, focus } = props;
  const timeZone = props.timeZone ?? "UTC";
  // The chart is laid out in the units it is shown in (its width on screen), so it can fill the page without its text
  // growing with it. Before it is measured (and where nothing can be) it is the default width.
  const figure = useRef<HTMLElement>(null);
  const [width, setWidth] = useState(WIDTH);
  useEffect(() => {
    const element = figure.current;
    if (element === null || typeof ResizeObserver === "undefined") return;
    const measure = () => {
      if (element.clientWidth > 0) setWidth(Math.max(MIN_WIDTH, Math.round(element.clientWidth)));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => { observer.disconnect(); };
  }, []);
  const [hover, setHover] = useState<number | null>(null);
  const drawn = layout(series, metric, width, timeZone);
  if (drawn.lines.length === 0) return <p class="status">No requests in this window.</p>;
  const choose = (group: string) => {
    props.onFocus(group === focus ? "" : group);
  };
  return (
    <figure class="chart" ref={figure}>
      <svg
        viewBox={`0 0 ${width.toString()} ${HEIGHT.toString()}`}
        role="img"
        aria-label={`${METRIC_LABELS[metric]} over time`}
        onPointerMove={(event) => {
          // The nearest bucket to the pointer, in the chart's own units; none outside the plot.
          const box = event.currentTarget.getBoundingClientRect();
          const x = (event.clientX - box.left) * (width / (box.width > 0 ? box.width : width));
          const points = drawn.lines[0]?.points ?? [];
          let nearest = -1;
          let best = Infinity;
          points.forEach((p, i) => {
            if (Math.abs(p.x - x) < best) {
              best = Math.abs(p.x - x);
              nearest = i;
            }
          });
          const inside = x >= LEFT - 8 && x <= width - RIGHT + 8;
          setHover(inside && nearest >= 0 ? nearest : null);
        }}
        onPointerLeave={() => { setHover(null); }}
      >
        {drawn.yTicks.map((t) => (
          <g key={t.value}>
            <line class="grid" x1={LEFT} x2={width - RIGHT} y1={t.y} y2={t.y} />
            <text class="axis" x={LEFT - 6} y={t.y + 4} text-anchor="end">{formatMetric(metric, t.value)}</text>
          </g>
        ))}
        <text class="axis" x={width - RIGHT} y={HEIGHT - 2} text-anchor="end">times are {timeZone}</text>
        {drawn.xTicks.map((t) => (
          <text key={t.x} class="axis" x={t.x} y={HEIGHT - BOTTOM + 16} text-anchor="middle">{t.label}</text>
        ))}
        {drawn.unreadTo !== null && (
          <g class="unread-region">
            <rect class="unread" x={LEFT} y={TOP} width={drawn.unreadTo - LEFT} height={HEIGHT - TOP - BOTTOM} />
            <text class="axis" x={LEFT + 4} y={TOP + 12}>not shown: the oldest requests were cut off</text>
          </g>
        )}
        {drawn.lines.map((line, i) => {
          const { solid, partial } = strokes(line.points);
          const last = line.points.filter((p) => p.y !== null).at(-1);
          const every = Math.max(1, Math.ceil(line.points.length / 12));
          return (
            <g key={line.group} class={`line s${(i % MAX_SERIES).toString()}${focus !== "" && focus !== line.group ? " dim" : ""}`}>
              {solid.map((d) => (
                <path key={d} class="stroke-line" d={d} />
              ))}
              {partial !== null && <path class="stroke-line partial" d={partial} />}
              {line.points.map((p, n) =>
                p.y !== null && (n % every === 0 || p === last) ? <Marker key={p.start_ms} shape={i} x={p.x} y={p.y} /> : null,
              )}
              {line.points.map((p) => (
                <circle key={p.start_ms} class="hit" cx={p.x} cy={p.y ?? TOP + (HEIGHT - TOP - BOTTOM)} r={6}>
                  <title>{pointTitle(line.group, metric, p, timeZone)}</title>
                </circle>
              ))}
              {last !== undefined && line.labelY !== null && (
                <text class="label" x={last.x + 8} y={line.labelY + 4}>{line.group.length > 14 ? `${line.group.slice(0, 13)}…` : line.group}</text>
              )}
            </g>
          );
        })}
        {hover !== null && <Tooltip drawn={drawn} index={hover} metric={metric} width={width} timeZone={timeZone} />}
      </svg>
      <ul class="legend">
        {drawn.lines.map((line, i) => (
          <li key={line.group}>
            <button type="button" class={`legend-item s${(i % MAX_SERIES).toString()}`} aria-pressed={focus === line.group} onClick={() => { choose(line.group); }}>
              <svg class="legend-mark" viewBox="0 0 12 12" aria-hidden="true">
                <Marker shape={i} x={6} y={6} />
              </svg>
              {line.group}
            </button>
          </li>
        ))}
      </ul>
      {series.truncated && <p role="status" class="notice">The gateway read only the newest decisions in this window, so the shaded part of the chart is not shown.</p>}
    </figure>
  );
}
