import { useState } from "preact/hooks";
import { clientSettings } from "./clients";

/**
 * Copy-paste settings for each tool, with the gateway's live address filled in. The text is shown as text, and the copy
 * button copies exactly what is shown.
 */
export function ConnectPanel(props: { gateway: string; routes: string[] }) {
  const all = clientSettings(props.gateway, props.routes);
  const [chosen, setChosen] = useState(all[0]?.id ?? "");
  const [copied, setCopied] = useState<"yes" | "no" | null>(null);
  const shown = all.find((s) => s.id === chosen) ?? all[0];
  if (shown === undefined) return null;

  function copy(text: string) {
    navigator.clipboard.writeText(text).then(
      () => {
        setCopied("yes");
      },
      () => {
        setCopied("no");
      },
    );
  }

  return (
    <section class="connect" aria-label="Connect a tool">
      <h2>Connect a tool</h2>
      <div class="tabs" role="tablist">
        {all.map((s) => (
          <button
            key={s.id}
            type="button"
            role="tab"
            aria-selected={s.id === shown.id}
            class={s.id === shown.id ? "tab on" : "tab"}
            onClick={() => {
              setChosen(s.id);
              setCopied(null);
            }}
          >
            {s.title}
          </button>
        ))}
      </div>
      <p class="hint">{shown.where}.</p>
      <pre class="settings">{shown.text}</pre>
      <div class="actions">
        <button type="button" onClick={() => { copy(shown.text); }}>Copy</button>
        {copied === "yes" && <span role="status" class="muted">Copied.</span>}
        {copied === "no" && <span role="status" class="error">Your browser did not allow copying; select the text instead.</span>}
      </div>
    </section>
  );
}
