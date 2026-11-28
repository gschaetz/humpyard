import { useState } from "preact/hooks";
import { ApiError } from "./api/client";
import type { KeySource, KeyStatus, LocalClient } from "./api/local";

const SOURCE_TEXT: Record<KeySource, string> = {
  environment: "has a key from your environment",
  file: "has a key saved on this machine",
  session: "has a key for this run only",
  none: "has no key",
};

/**
 * A write-only form for one provider's key. The field is emptied the moment the key is sent, whatever the answer, and the
 * page never shows, stores or asks for a key back: it only learns whether a provider has one and where it comes from.
 */
function KeyRow(props: { status: KeyStatus; local: LocalClient; onChanged: () => void }) {
  const { status, local } = props;
  const [value, setValue] = useState("");
  const [save, setSave] = useState(false);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<{ text: string; bad: boolean } | null>(null);

  async function send(event: Event) {
    event.preventDefault();
    const key = value.trim();
    if (key === "") return;
    // Out of the field before anything is awaited.
    setValue("");
    setBusy(true);
    setNote(null);
    try {
      setNote({ text: await local.setKey(status.provider, key, save), bad: false });
      props.onChanged();
    } catch (failure) {
      setNote({ text: failure instanceof ApiError ? failure.message : "The key could not be sent.", bad: true });
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    setBusy(true);
    setNote(null);
    try {
      setNote({ text: await local.removeKey(status.provider), bad: false });
      props.onChanged();
    } catch (failure) {
      setNote({ text: failure instanceof ApiError ? failure.message : "The key could not be removed.", bad: true });
    } finally {
      setBusy(false);
    }
  }

  const name = `${status.provider}-key`;
  return (
    <form class="key-row" onSubmit={(event) => void send(event)}>
      <h3>
        {status.provider} <span class="muted">{SOURCE_TEXT[status.source]}</span>
      </h3>
      <label>
        Key for {status.provider}
        <input
          type="password"
          name={name}
          autocomplete="off"
          spellcheck={false}
          value={value}
          onInput={(event) => {
            setValue(event.currentTarget.value);
          }}
        />
      </label>
      <fieldset class="choices">
        <legend class="muted">What to do with it</legend>
        <label>
          <input type="radio" name={`${name}-keep`} checked={!save} onChange={() => { setSave(false); }} /> Use for this run only
        </label>
        <label>
          <input type="radio" name={`${name}-keep`} checked={save} onChange={() => { setSave(true); }} /> Save on this machine
        </label>
      </fieldset>
      <div class="actions">
        <button type="submit" disabled={busy}>Use this key</button>
        {status.source === "file" || status.source === "session" ? (
          <button type="button" disabled={busy} onClick={() => void remove()}>Remove</button>
        ) : null}
      </div>
      {note !== null && (
        <p role={note.bad ? "alert" : "status"} class={note.bad ? "error" : "muted"}>{note.text}</p>
      )}
    </form>
  );
}

/** Every configured provider's key form, or the reason there is none. */
export function KeyForm(props: { local: LocalClient; keyEntry: boolean; statuses: KeyStatus[]; onChanged: () => void }) {
  if (!props.keyEntry) {
    return (
      <section class="keys" aria-label="Provider keys">
        <h2>Provider keys</h2>
        <p class="muted">
          Keys cannot be entered here because this page is served beyond this machine. Set them in the gateway's
          environment instead.
        </p>
      </section>
    );
  }
  return (
    <section class="keys" aria-label="Provider keys">
      <h2>Provider keys</h2>
      <p class="hint">A key you send is checked with its provider first. It is never shown again.</p>
      {props.statuses.map((status) => (
        <KeyRow key={status.provider} status={status} local={props.local} onChanged={props.onChanged} />
      ))}
    </section>
  );
}
