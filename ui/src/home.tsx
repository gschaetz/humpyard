import { useCallback, useEffect, useState } from "preact/hooks";
import type { Client } from "./api/client";
import type { ConnectInfo, KeyStatus, LocalClient, TestResult } from "./api/local";
import { ConnectPanel } from "./connect-panel";
import { EmptyState } from "./empty-state";
import { KeyForm } from "./key-form";
import { QueryView } from "./query-view";

/** Whether the gateway has answered a call: `null` while it is not known, and always `null` for a gateway that is down. */
async function answeredAny(client: Client): Promise<boolean | null> {
  try {
    const stats = await client.stats({ window: "30d", groupBy: "target" });
    return stats.data.some((group) => group.answered > 0);
  } catch {
    return null;
  }
}

/**
 * What `humpyard up` opens on. With no gateway running (no key yet) it asks for keys; with a gateway that has answered
 * nothing it shows how to connect a tool and offers a test request; once a call has been answered it shows the query view,
 * with the savings comparison chosen for the person, and keeps the connect settings and the keys one fold away.
 */
export function UpHome(props: {
  client: Client;
  local: LocalClient;
  info: ConnectInfo;
  onRefused: () => void;
  /** A key changed or a test ran: the shell reads the gateway again. */
  onChanged: () => void;
}) {
  const { client, local, info } = props;
  // `undefined` until the first read has come back, so the page does not show one screen and then swap it for another;
  // `null` when the read failed, which the query view then reports in its own words.
  const [answered, setAnswered] = useState<boolean | null | undefined>(undefined);
  const [statuses, setStatuses] = useState<KeyStatus[]>([]);
  // A test request that worked: it is an answered call, and the statistics (shared for ten seconds) may not show it yet.
  const [tested, setTested] = useState<TestResult | null>(null);

  const readAnswered = useCallback(() => {
    if (!info.gateway_running) return;
    void answeredAny(client).then(setAnswered);
  }, [client, info.gateway_running]);

  const readKeys = useCallback(() => {
    local.keys().then(
      (keys) => {
        setStatuses(keys.providers);
      },
      () => {
        setStatuses([]);
      },
    );
  }, [local]);

  useEffect(() => {
    readAnswered();
    readKeys();
  }, [readAnswered, readKeys]);

  const changed = () => {
    readKeys();
    props.onChanged();
  };
  const keys = <KeyForm local={local} keyEntry={info.key_entry} statuses={statuses} onChanged={changed} />;

  if (!info.gateway_running) {
    return (
      <div class="home">
        <section class="empty" aria-label="The gateway is not running">
          <h2>Add a key to start the gateway</h2>
          <p>
            humpyard needs a key for at least one provider. Add one below; it is checked with the provider before anything
            starts.
          </p>
        </section>
        {keys}
      </div>
    );
  }
  if (answered === undefined) return <p class="status">Looking at the gateway…</p>;
  if (answered === false) {
    return (
      <div class="home">
        <EmptyState
          local={local}
          onResult={(result) => {
            if (!result.ok) return;
            setTested(result);
            setAnswered(true);
          }}
        />
        <ConnectPanel gateway={info.gateway} routes={info.routes} />
        <details>
          <summary>Provider keys</summary>
          {keys}
        </details>
      </div>
    );
  }
  return (
    <div class="home">
      {tested !== null && (
        <p role="status" class="ok">
          The test request worked: <strong>{tested.target ?? tested.model}</strong> answered in {String(tested.latency_ms ?? 0)} ms.
        </p>
      )}
      <QueryView client={client} onRefused={props.onRefused} preferBaseline />
      <details>
        <summary>Connect a tool</summary>
        <ConnectPanel gateway={info.gateway} routes={info.routes} />
      </details>
      <details>
        <summary>Provider keys</summary>
        {keys}
      </details>
    </div>
  );
}
