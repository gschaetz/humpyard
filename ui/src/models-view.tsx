import { useEffect, useState } from "preact/hooks";
import { ApiError, type Client } from "./api/client";
import { count, ms, percent, tokens, usd } from "./format";
import { buildRows, overallState, signalRate, SIGNALS, WINDOWS, type ModelRow, type Window } from "./models";

type Load =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "ready"; rows: ModelRow[]; truncated: boolean };

/**
 * Each target and how it is doing over the chosen window, from `/admin/v1/stats` and `/admin/v1/health` only. Every
 * string in a row (a target or provider name, a model) came from the gateway's configuration or from clients and is
 * rendered as text. A failed load replaces the table with the error: the last numbers are never left on screen as if
 * they were current.
 */
export function ModelsView(props: { client: Client }) {
  const { client } = props;
  const [window, setWindow] = useState<Window>("24h");
  const [load, setLoad] = useState<Load>({ kind: "loading" });
  const [reload, setReload] = useState(0);

  useEffect(() => {
    let current = true;
    setLoad({ kind: "loading" });
    Promise.all([client.stats({ window, groupBy: "target" }), client.health()])
      .then(([stats, health]) => {
        if (current) setLoad({ kind: "ready", rows: buildRows(stats, health), truncated: stats.truncated });
      })
      .catch((failure: unknown) => {
        if (current) {
          setLoad({
            kind: "error",
            message: failure instanceof ApiError ? failure.message : "The statistics could not be read.",
          });
        }
      });
    return () => {
      current = false;
    };
  }, [client, window, reload]);

  return (
    <section class="models">
      <div class="toolbar">
        <h2>Models</h2>
        <label>
          Window
          <select
            value={window}
            onChange={(event) => {
              setWindow(event.currentTarget.value as Window);
            }}
          >
            {WINDOWS.map((w) => (
              <option key={w} value={w}>{w}</option>
            ))}
          </select>
        </label>
        <button type="button" onClick={() => { setReload((n) => n + 1); }}>Refresh</button>
      </div>
      {load.kind === "loading" && <p class="status">Loading…</p>}
      {load.kind === "error" && <p role="alert" class="error">{load.message}</p>}
      {load.kind === "ready" && <Table rows={load.rows} truncated={load.truncated} />}
    </section>
  );
}

function Table(props: { rows: ModelRow[]; truncated: boolean }) {
  if (props.rows.length === 0) return <p class="status">No targets.</p>;
  return (
    <>
      {props.truncated && <p role="status" class="notice">The statistics were cut short; the figures below may be incomplete.</p>}
      <table>
        <thead>
          <tr>
            <th>Target</th>
            <th>Health</th>
            <th class="num">Requests</th>
            <th class="num">Answered</th>
            <th class="num">Failed</th>
            <th class="num">Fell back</th>
            {SIGNALS.map((s) => (
              <th key={s} class="num">{s.replace("_", " ")}</th>
            ))}
            <th class="num">Tokens in / out</th>
            <th class="num">Avg cost</th>
            <th class="num">p50</th>
            <th class="num">p90</th>
            <th class="num">p99</th>
          </tr>
        </thead>
        <tbody>
          {props.rows.map((row) => (
            <Row key={row.group} row={row} />
          ))}
        </tbody>
      </table>
    </>
  );
}

function Row(props: { row: ModelRow }) {
  const { row } = props;
  const g = row.stats;
  return (
    <tr>
      <th scope="row" class="target">{row.group}</th>
      <td><Health row={row} /></td>
      <td class="num">{g ? count(g.requests) : "0"}</td>
      <td class="num">{g ? count(g.answered) : "0"}</td>
      <td class="num">{g ? percent(g.failure_rate) : percent(null)}</td>
      <td class="num">{g ? percent(g.fallback_rate) : percent(null)}</td>
      {SIGNALS.map((s) => (
        <td key={s} class="num">{g ? percent(signalRate(g, s)) : percent(null)}</td>
      ))}
      <td class="num">{g ? `${tokens(g.tokens.input)} / ${tokens(g.tokens.output)}` : "—"}</td>
      <td class="num">{g ? usd(g.avg_cost_usd) : "—"}</td>
      <td class="num">{g ? ms(g.latency_ms.p50) : "—"}</td>
      <td class="num">{g ? ms(g.latency_ms.p90) : "—"}</td>
      <td class="num">{g ? ms(g.latency_ms.p99) : "—"}</td>
    </tr>
  );
}

function Health(props: { row: ModelRow }) {
  const state = overallState(props.row.endpoints);
  const label = { healthy: "healthy", probing: "recovering", cooling_down: "cooling down", none: "no endpoints" }[state];
  const detail = props.row.endpoints
    .map((e) => `${e.provider}/${e.model}: ${e.state.replace("_", " ")}`)
    .join("\n");
  return (
    <span class={`health ${state}`} title={detail}>
      {label}
      {props.row.currentLatencyMs !== null && <span class="muted"> · {ms(props.row.currentLatencyMs)} now</span>}
    </span>
  );
}
