import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import type { Client, Health, Series, Stats } from "./api/client";
import { Bars } from "./bars";
import { LineChart } from "./chart";
import { timeZoneId } from "./chart-data";
import { clock } from "./format";
import { buildRows, type ModelRow } from "./models";
import { QueryBar, type View } from "./query-bar";
import { INTERVALS, REFRESH_MS, parseQuery, serialiseQuery, type Query } from "./query";
import { Refresher, type Failure } from "./refresh";
import { Table } from "./table";

/** What one read brings back; `series` is null when the gateway has no series endpoint. */
interface Data {
  series: Series | null;
  stats: Stats;
  health: Health;
}

/** The most series asked for: also what the chart can tell apart. */
const SERIES_LIMIT = 8;

/** The query in the URL fragment, and a way to change it. Nothing else is kept anywhere. */
function useFragmentQuery(): [Query, (change: Partial<Query>) => void] {
  const [query, setQuery] = useState(() => parseQuery(window.location.hash));
  useEffect(() => {
    const onHash = () => { setQuery(parseQuery(window.location.hash)); };
    window.addEventListener("hashchange", onHash);
    return () => { window.removeEventListener("hashchange", onHash); };
  }, []);
  const change = (patch: Partial<Query>) => {
    setQuery((current) => {
      const next = { ...current, ...patch };
      window.history.replaceState(null, "", `#${serialiseQuery(next)}`);
      return next;
    });
  };
  return [query, change];
}

/**
 * The query view: a query bar, the chosen metric over time (or as a ranking), and a table of the groups, from
 * `/admin/v1/stats`, `/admin/v1/stats/series` and `/admin/v1/health` only. It reads again when the query changes and, on
 * the chosen period, while the page is visible. Every name in it came from the gateway's configuration or from clients
 * and is rendered as text. After a failed refresh the earlier numbers stay, marked out of date with the time they are from;
 * nothing is left looking current.
 */
export function QueryView(props: { client: Client; onRefused: () => void }) {
  const { client } = props;
  const [query, change] = useFragmentQuery();
  const [view, setView] = useState<View>("lines");
  const [data, setData] = useState<{ data: Data; at: number } | null>(null);
  const [failure, setFailure] = useState<Failure | null>(null);
  const [loading, setLoading] = useState(true);

  // The reader always sees the latest query without being rebuilt for it.
  const current = useRef(query);
  current.current = query;
  const onRefused = useRef(props.onRefused);
  onRefused.current = props.onRefused;

  const refresher = useMemo(
    () =>
      new Refresher<Data>({
        load: async (signal) => {
          const q = current.current;
          const filters = { route: q.route === "" ? undefined : q.route, target: q.target === "" ? undefined : q.target };
          // The series and the statistics are both heavy reads and the gateway allows one at a time on this surface,
          // so they are asked one after the other (a parallel pair would have the second refused as busy). Health is a
          // light read and goes with the first.
          const [series, health] = await Promise.all([
            client.series({ window: q.window, interval: INTERVALS[q.window], groupBy: q.group, limit: SERIES_LIMIT, ...filters }, { signal }),
            client.health({ signal }),
          ]);
          const stats = await client.stats({ window: q.window, groupBy: q.group, ...filters }, { signal });
          return { series, stats, health };
        },
        onData: (next, at) => {
          setData({ data: next, at });
          setFailure(null);
        },
        onFailure: setFailure,
        onRefused: () => { onRefused.current(); },
        onRunning: setLoading,
      }),
    [client],
  );

  // A change of anything that is asked of the gateway reads again at once and starts the period over.
  const asked = [query.window, query.group, query.metric, query.route, query.target, query.refresh].join("|");
  useEffect(() => {
    refresher.setPeriod(REFRESH_MS[current.current.refresh]);
    refresher.restart();
  }, [refresher, asked]);
  useEffect(() => {
    const onVisibility = () => { refresher.visibilityChanged(); };
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      refresher.stop();
    };
  }, [refresher]);

  const rows = useMemo(() => {
    if (data === null) return [];
    let built = buildRows(data.data.stats, data.data.health, query.group);
    if (query.route !== "" || query.target !== "") built = built.filter((r) => r.stats !== null);
    return built;
  }, [data, query.group, query.route, query.target]);
  const shown: ModelRow[] = query.focus === "" ? rows : rows.filter((r) => r.group === query.focus);
  const filtered = query.route !== "" || query.target !== "";

  return (
    <section class="models">
      <QueryBar
        query={query}
        view={view}
        onQuery={change}
        onView={setView}
        onRefreshNow={() => { refresher.restart(); }}
      />
      <Status data={data} failure={failure} loading={loading} period={query.refresh} />
      {data !== null && (
        <div class={failure === null ? "" : "stale"}>
          {data.data.stats.truncated && (
            <p role="status" class="notice">The statistics were cut short; the figures below may be incomplete.</p>
          )}
          {rows.length === 0 ? (
            <p class="status">{filtered ? "Nothing matched this query." : "No targets."}</p>
          ) : (
            <>
              {view === "bars" ? (
                <Bars rows={shown} metric={query.metric} focus={query.focus} onFocus={(focus) => { change({ focus }); }} />
              ) : data.data.series === null ? (
                <p role="status" class="notice">
                  This gateway cannot draw a series: it is older than this page. The table below is current.
                </p>
              ) : (
                <LineChart series={data.data.series} metric={query.metric} focus={query.focus} timeZone={timeZoneId(query.zone)} onFocus={(focus) => { change({ focus }); }} />
              )}
              <Table rows={shown} group={query.group} />
            </>
          )}
        </div>
      )}
    </section>
  );
}

function Status(props: { data: { at: number } | null; failure: Failure | null; loading: boolean; period: Query["refresh"] }) {
  const { data, failure } = props;
  if (data === null && failure === null) return <p class="status">Loading…</p>;
  return (
    <div class="refresh-status">
      {failure !== null && (
        <p role="alert" class="error">
          {failure.message}
          {data !== null && ` The numbers below are out of date: they are from ${clock(data.at)}.`}
        </p>
      )}
      {data !== null && failure === null && (
        <p class="muted" aria-live="off">
          Updated {clock(data.at)}
          {props.period === "off" ? " · not refreshing" : ` · every ${props.period}`}
          {props.loading ? " · updating…" : ""}
        </p>
      )}
    </div>
  );
}
