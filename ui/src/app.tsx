import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import { ApiError, createClient, type Client, type Principal } from "./api/client";
import { createLocalClient, type ConnectInfo, type LocalClient } from "./api/local";
import { clearCredential, loadCredential } from "./api/session";
import { contractNotice } from "./api/version";
import { UpHome } from "./home";
import { takeTokenFromLocation } from "./link-token";
import { QueryView } from "./query-view";
import { SignIn } from "./signin";
import { TokenEntry } from "./token";

type Phase =
  | { kind: "checking" }
  | { kind: "signin" }
  | { kind: "token" }
  | { kind: "unreachable"; message: string }
  | { kind: "ready"; principal: Principal; up: ConnectInfo | null }
  // Served by `humpyard up` with no gateway answering (no key yet): the page can still ask for one.
  | { kind: "setup"; up: ConnectInfo };

/**
 * The shell: finds out whether the API will talk to us (a credential held for this session, or one that
 * `humpyard ui` injects), asks for one when it will not, and checks the contract version before showing any data.
 * `client` is for tests; the page builds its own, which reads the credential this component holds.
 */
export function App(props: { client?: Client; local?: LocalClient }) {
  const held = useRef<string | null>(loadCredential());
  const client = useMemo(() => props.client ?? createClient(() => held.current), [props.client]);
  const local = useMemo(() => props.local ?? createLocalClient(), [props.local]);
  const [phase, setPhase] = useState<Phase>({ kind: "checking" });
  const [notice, setNotice] = useState<string | null>(null);
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let current = true;
    setPhase({ kind: "checking" });
    client
      .whoami()
      .then(async (principal) => {
        // A version we cannot read is reported, not fatal: the data below is still worth showing.
        const gateway = await client.contractVersion().then(
          (version) => contractNotice(version),
          () => "This gateway's admin API version could not be read.",
        );
        // Under `humpyard up` there is more to show; anywhere else this is `null` and the page is as it was.
        const up = await local.connect();
        if (!current) return;
        setNotice(gateway);
        setPhase({ kind: "ready", principal, up });
      })
      .catch((failure: unknown) => {
        if (!current) return;
        if (failure instanceof ApiError && failure.code === "ui_token_required") {
          // `humpyard ui` holds the credential and wants its access token first. `humpyard up` opens the page with
          // the token in the address, taken out of it before it is used; anything else asks for it.
          const linked = takeTokenFromLocation();
          if (linked === null) {
            setPhase({ kind: "token" });
          } else {
            client.exchangeToken(linked).then(
              () => {
                if (current) setAttempt((n) => n + 1);
              },
              () => {
                if (current) setPhase({ kind: "token" });
              },
            );
          }
        } else if (failure instanceof ApiError && (failure.status === 401 || failure.status === 403)) {
          setPhase({ kind: "signin" });
        } else {
          const message = failure instanceof ApiError ? failure.message : "The admin API could not be reached.";
          // Under `humpyard up` a gateway that is not running (it has no key yet) is something the page can fix.
          void local.connect().then((up) => {
            if (!current) return;
            setPhase(up === null ? { kind: "unreachable", message } : { kind: "setup", up });
          });
        }
      });
    return () => {
      current = false;
    };
  }, [client, local, attempt]);

  // Back through the checks that decide between sign-in, token, setup and the page itself.
  function refused() {
    setAttempt((n) => n + 1);
  }

  function signOut() {
    clearCredential();
    held.current = null;
    setNotice(null);
    setPhase({ kind: "signin" });
  }

  switch (phase.kind) {
    case "checking":
      return <p class="status">Connecting…</p>;
    case "signin":
      return (
        <SignIn
          client={client}
          hold={(credential) => {
            held.current = credential === "" ? null : credential;
          }}
          onSignedIn={() => {
            setAttempt((n) => n + 1);
          }}
        />
      );
    case "token":
      return (
        <TokenEntry
          client={client}
          onDone={() => {
            setAttempt((n) => n + 1);
          }}
        />
      );
    case "unreachable":
      return (
        <div class="status">
          <p role="alert" class="error">{phase.message}</p>
          <button type="button" onClick={() => { setAttempt((n) => n + 1); }}>Try again</button>
        </div>
      );
    case "ready":
      return (
        <div class="shell">
          <header>
            <strong>humpyard admin</strong>
            <span class="who">{phase.principal.id}</span>
            <button type="button" onClick={signOut}>Sign out</button>
          </header>
          {notice !== null && <p role="status" class="notice">{notice}</p>}
          <main>
            {phase.up === null ? (
              <QueryView client={client} onRefused={refused} />
            ) : (
              <UpHome client={client} local={local} info={phase.up} onRefused={refused} onChanged={refused} />
            )}
          </main>
        </div>
      );
    case "setup":
      return (
        <div class="shell">
          <header>
            <strong>humpyard</strong>
          </header>
          <main>
            <UpHome client={client} local={local} info={phase.up} onRefused={refused} onChanged={refused} />
          </main>
        </div>
      );
  }
}
