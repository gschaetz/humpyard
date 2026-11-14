import { useEffect, useState } from "preact/hooks";
import { perMillion, type Offered } from "./baseline";
import { METRIC_LABELS } from "./chart-data";
import { GROUPS, METRICS, REFRESHES, WINDOWS, ZONES, type Group, type Metric, type Query, type Refresh, type Window, type Zone } from "./query";

export type View = "lines" | "bars";

const GROUP_LABELS: Record<Group, string> = { route: "Route", target: "Target", endpoint: "Endpoint" };
const ZONE_LABELS: Record<Zone, string> = { local: "Local time", utc: "UTC" };
const REFRESH_LABELS: Record<Refresh, string> = { off: "Off", "30s": "30 seconds", "1m": "1 minute", "5m": "5 minutes" };

/** A text filter that changes the query when the person commits it (Enter or leaving the field), not on each key. */
function Filter(props: { label: string; value: string; onCommit: (value: string) => void }) {
  const [draft, setDraft] = useState(props.value);
  useEffect(() => {
    setDraft(props.value);
  }, [props.value]);
  const commit = () => {
    if (draft !== props.value) props.onCommit(draft);
  };
  return (
    <label>
      {props.label}
      <input
        type="search"
        value={draft}
        maxLength={200}
        placeholder="any"
        onInput={(event) => { setDraft(event.currentTarget.value); }}
        onChange={commit}
        onKeyDown={(event) => {
          if (event.key === "Enter") commit();
        }}
      />
    </label>
  );
}

/**
 * The query bar: what to show (window, grouping, metric, route and target filters), how (lines or ranking, local time
 * or UTC) and how often to refresh. Every choice comes from a fixed list; the filters are plain text. Changing any of them asks the
 * view to read again.
 */
export function QueryBar(props: {
  query: Query;
  view: View;
  onQuery: (change: Partial<Query>) => void;
  onView: (view: View) => void;
  onRefreshNow: () => void;
  /** The endpoints the traffic can be compared with; none hides the choice. */
  offered: Offered[];
  /** The baseline in use: one of `offered`, or empty for none. */
  baseline: string;
}) {
  const { query } = props;
  return (
    <div class="query-bar toolbar">
      <label>
        Window
        <select value={query.window} onChange={(event) => { props.onQuery({ window: event.currentTarget.value as Window, focus: "" }); }}>
          {WINDOWS.map((w) => (
            <option key={w} value={w}>{w}</option>
          ))}
        </select>
      </label>
      <label>
        Group by
        <select value={query.group} onChange={(event) => { props.onQuery({ group: event.currentTarget.value as Group, focus: "" }); }}>
          {GROUPS.map((g) => (
            <option key={g} value={g}>{GROUP_LABELS[g]}</option>
          ))}
        </select>
      </label>
      <label>
        Metric
        <select value={query.metric} onChange={(event) => { props.onQuery({ metric: event.currentTarget.value as Metric }); }}>
          {METRICS.map((m) => (
            <option key={m} value={m}>{METRIC_LABELS[m]}</option>
          ))}
        </select>
      </label>
      <Filter label="Route" value={query.route} onCommit={(route) => { props.onQuery({ route, focus: "" }); }} />
      <Filter label="Target" value={query.target} onCommit={(target) => { props.onQuery({ target, focus: "" }); }} />
      <label>
        Show
        <select value={props.view} onChange={(event) => { props.onView(event.currentTarget.value as View); }}>
          <option value="lines">Over time</option>
          <option value="bars">Ranking</option>
        </select>
      </label>
      {props.offered.length > 0 && (
        <label>
          Compare with
          <select value={props.baseline} onChange={(event) => { props.onQuery({ baseline: event.currentTarget.value }); }}>
            <option value="">None</option>
            {props.offered.map((o) => (
              <option key={o.name} value={o.name}>{`${o.name} (${perMillion(o.input)} in, ${perMillion(o.output)} out)`}</option>
            ))}
          </select>
        </label>
      )}
      <label>
        Times
        <select value={query.zone} onChange={(event) => { props.onQuery({ zone: event.currentTarget.value as Zone }); }}>
          {ZONES.map((z) => (
            <option key={z} value={z}>{ZONE_LABELS[z]}</option>
          ))}
        </select>
      </label>
      <label>
        Refresh
        <select value={query.refresh} onChange={(event) => { props.onQuery({ refresh: event.currentTarget.value as Refresh }); }}>
          {REFRESHES.map((r) => (
            <option key={r} value={r}>{REFRESH_LABELS[r]}</option>
          ))}
        </select>
      </label>
      <button type="button" onClick={props.onRefreshNow}>Refresh now</button>
    </div>
  );
}
