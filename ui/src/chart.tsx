import { useEffect, useRef, useState } from "preact/hooks";
import type { Series } from "./api/client";
import { BOTTOM, HEIGHT, LEFT, MAX_SERIES, METRIC_LABELS, MIN_WIDTH, RIGHT, TOP, WIDTH, formatMetric, layout, pointTitle, strokes } from "./chart-data";
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

/**
 * The metric over time, one line per group, drawn as SVG from the answer. All colours and strokes come from classes
 * in the stylesheet (`s0` to `s7`, `partial`, `dim`) and all geometry from attributes: no `style` attribute is written,
 * so the page's Content-Security-Policy is untouched. Every name in it is text. A bucket with no value is a gap and
 * says so on hover; the last bucket is dashed because it is still filling; a window the row cap cut is shaded and said so.
 * The legend is the filter control: choosing a series dims the others and (in the view) filters the table to it.
 */
export function LineChart(props: { series: Series; metric: Metric; focus: string; onFocus: (group: string) => void }) {
  const { series, metric, focus } = props;
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
  const drawn = layout(series, metric, width);
  if (drawn.lines.length === 0) return <p class="status">No requests in this window.</p>;
  const choose = (group: string) => {
    props.onFocus(group === focus ? "" : group);
  };
  return (
    <figure class="chart" ref={figure}>
      <svg viewBox={`0 0 ${width.toString()} ${HEIGHT.toString()}`} role="img" aria-label={`${METRIC_LABELS[metric]} over time`}>
        {drawn.yTicks.map((t) => (
          <g key={t.value}>
            <line class="grid" x1={LEFT} x2={width - RIGHT} y1={t.y} y2={t.y} />
            <text class="axis" x={LEFT - 6} y={t.y + 4} text-anchor="end">{formatMetric(metric, t.value)}</text>
          </g>
        ))}
        <text class="axis" x={width - RIGHT} y={HEIGHT - 2} text-anchor="end">times are UTC</text>
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
                  <title>{pointTitle(line.group, metric, p)}</title>
                </circle>
              ))}
              {last !== undefined && line.labelY !== null && (
                <text class="label" x={last.x + 8} y={line.labelY + 4}>{line.group.length > 14 ? `${line.group.slice(0, 13)}…` : line.group}</text>
              )}
            </g>
          );
        })}
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
