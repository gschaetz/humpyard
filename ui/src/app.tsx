import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import { ApiError, createClient, type Client, type Principal } from "./api/client";
import { clearCredential, loadCredential } from "./api/session";
import { contractNotice } from "./api/version";
import { ModelsView } from "./models-view";
import { SignIn } from "./signin";

type Phase =
  | { kind: "checking" }
  | { kind: "signin" }
  | { kind: "unreachable"; message: string }
  | { kind: "ready"; principal: Principal };

/**
 * The shell: finds out whether the API will talk to us (a credential held for this session, or one that
 * `humpyard ui` injects), asks for one when it will not, and checks the contract version before showing any data.
 * `client` is for tests; the page builds its own, which reads the credential this component holds.
 */
export function App(props: { client?: Client }) {
  const held = useRef<string | null>(loadCredential());
  const client = useMemo(() => props.client ?? createClient(() => held.current), [props.client]);
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
        if (!current) return;
        setNotice(gateway);
        setPhase({ kind: "ready", principal });
      })
      .catch((failure: unknown) => {
        if (!current) return;
        if (failure instanceof ApiError && (failure.status === 401 || failure.status === 403)) {
          setPhase({ kind: "signin" });
        } else {
          setPhase({
            kind: "unreachable",
            message: failure instanceof ApiError ? failure.message : "The admin API could not be reached.",
          });
        }
      });
    return () => {
      current = false;
    };
  }, [client, attempt]);

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
            <ModelsView client={client} />
          </main>
        </div>
      );
  }
}
