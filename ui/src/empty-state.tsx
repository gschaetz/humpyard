import { useState } from "preact/hooks";
import type { LocalClient, TestResult } from "./api/local";

/**
 * Shown while the gateway has answered nothing: says so in words, names the next step, and can send one small request to
 * prove the path, showing which target answered and how fast, or the step that failed.
 */
export function EmptyState(props: { local: LocalClient; onResult: (result: TestResult) => void }) {
  const [result, setResult] = useState<TestResult | null>(null);
  const [busy, setBusy] = useState(false);

  async function run() {
    setBusy(true);
    let outcome: TestResult;
    try {
      outcome = await props.local.test();
    } catch {
      outcome = { ok: false, step: "page", message: "The test request could not be made." };
    }
    setResult(outcome);
    setBusy(false);
    props.onResult(outcome);
  }

  return (
    <section class="empty" aria-label="Nothing yet">
      <h2>Nothing has gone through the gateway yet</h2>
      <p>
        Connect a tool below and use it, or send a test request now. What comes back, and how it was routed, appears here.
      </p>
      <button type="button" disabled={busy} onClick={() => void run()}>Send a test request</button>
      {result !== null && result.ok && (
        <p role="status" class="ok">
          It worked: <strong>{result.target ?? result.model}</strong> answered in {String(result.latency_ms ?? 0)} ms.
        </p>
      )}
      {result !== null && !result.ok && (
        <p role="alert" class="error">{result.message ?? "The test request failed."}</p>
      )}
    </section>
  );
}
