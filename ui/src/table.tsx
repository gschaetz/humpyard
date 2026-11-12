import { useState } from "preact/hooks";
import { count, ms, percent, tokens, usd } from "./format";
import { SIGNALS, overallState, signalRate, sortRows, type ModelRow, type Sort, type SortKey } from "./models";
import type { Group } from "./query";

const NAME: Record<Group, string> = { route: "Route", target: "Target", endpoint: "Endpoint" };

function Health(props: { row: ModelRow }) {
  const state = overallState(props.row.endpoints);
  const label = { healthy: "healthy", probing: "recovering", cooling_down: "cooling down", none: "no endpoints" }[state];
  const detail = props.row.endpoints.map((e) => `${e.provider}/${e.model}: ${e.state.replace("_", " ")}`).join("\n");
  return (
    <span class={`health ${state}`} title={detail}>
      {label}
      {props.row.currentLatencyMs !== null && <span class="muted"> · {ms(props.row.currentLatencyMs)} now</span>}
    </span>
  );
}

function Header(props: { label: string; by: SortKey; sort: Sort; numeric?: boolean; onSort: (by: SortKey) => void }) {
  const active = props.sort.by === props.by;
  return (
    <th class={props.numeric ? "num" : ""} aria-sort={active ? (props.sort.descending ? "descending" : "ascending") : "none"}>
      <button type="button" class="sort" onClick={() => { props.onSort(props.by); }}>
        {props.label}
        {active ? (props.sort.descending ? " ↓" : " ↑") : ""}
      </button>
    </th>
  );
}

/**
 * The groups with four figures each, problems first unless a column is chosen: health (where the grouping has any),
 * requests, failure rate, fallback rate and p90 latency. Tokens, cost, answered calls, the other percentiles and the
 * response-signal rates are in the detail row. Names and models are text. A rate with nothing behind it is a dash,
 * never 0%.
 */
export function Table(props: { rows: ModelRow[]; group: Group }) {
  const [sort, setSort] = useState<Sort>({ by: "default", descending: false });
  const [open, setOpen] = useState<Set<string>>(new Set());
  const rows = sortRows(props.rows, sort);
  const hasHealth = rows.some((r) => r.hasHealth);
  const columns = hasHealth ? 6 : 5;
  const choose = (by: SortKey) => {
    setSort((s) => ({ by, descending: s.by === by ? !s.descending : by !== "group" }));
  };
  const toggle = (name: string) => {
    setOpen((current) => {
      const next = new Set(current);
      if (!next.delete(name)) next.add(name);
      return next;
    });
  };
  return (
    <div class="table-wrap">
      {sort.by !== "default" && (
        <button type="button" class="default-order" onClick={() => { setSort({ by: "default", descending: false }); }}>
          Default order (problems first)
        </button>
      )}
      <table>
        <thead>
          <tr>
            <th aria-label="Details" />
            <Header label={NAME[props.group]} by="group" sort={sort} onSort={choose} />
            {hasHealth && <Header label="Health" by="health" sort={sort} onSort={choose} />}
            <Header label="Requests" by="requests" sort={sort} numeric onSort={choose} />
            <Header label="Failed" by="failure_rate" sort={sort} numeric onSort={choose} />
            <Header label="Fell back" by="fallback_rate" sort={sort} numeric onSort={choose} />
            <Header label="p90" by="latency" sort={sort} numeric onSort={choose} />
          </tr>
        </thead>
        {rows.map((row) => {
          const g = row.stats;
          const expanded = open.has(row.group);
          return (
            <tbody key={row.group}>
              <tr>
                <td>
                  <button type="button" class="expand" aria-expanded={expanded} aria-label={`Details of ${row.group}`} onClick={() => { toggle(row.group); }}>
                    {expanded ? "▾" : "▸"}
                  </button>
                </td>
                <th scope="row" class="target">{row.group}</th>
                {hasHealth && <td>{row.hasHealth ? <Health row={row} /> : <span class="muted">—</span>}</td>}
                <td class="num">{g ? count(g.requests) : "0"}</td>
                <td class="num">{g ? percent(g.failure_rate) : percent(null)}</td>
                <td class="num">{g ? percent(g.fallback_rate) : percent(null)}</td>
                <td class="num">{g ? ms(g.latency_ms.p90) : "—"}</td>
              </tr>
              {expanded && (
                <tr class="detail">
                  <td colSpan={columns + 1}>
                    <dl>
                      <dt>Answered</dt>
                      <dd>{g ? count(g.answered) : "—"}</dd>
                      <dt>Tokens in / out</dt>
                      <dd>{g ? `${tokens(g.tokens.input)} / ${tokens(g.tokens.output)}` : "—"}</dd>
                      <dt>Avg cost</dt>
                      <dd>{g ? usd(g.avg_cost_usd) : "—"}</dd>
                      <dt>p50</dt>
                      <dd>{g ? ms(g.latency_ms.p50) : "—"}</dd>
                      <dt>p99</dt>
                      <dd>{g ? ms(g.latency_ms.p99) : "—"}</dd>
                      {SIGNALS.map((s) => (
                        <>
                          <dt key={`${s}-t`}>{s.replace("_", " ")}</dt>
                          <dd key={`${s}-d`}>{g ? percent(signalRate(g, s)) : percent(null)}</dd>
                        </>
                      ))}
                    </dl>
                  </td>
                </tr>
              )}
            </tbody>
          );
        })}
      </table>
    </div>
  );
}
