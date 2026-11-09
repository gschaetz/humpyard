import { useState } from "preact/hooks";
import type { Client } from "./api/client";
import { ApiError } from "./api/client";

/**
 * Asks for the access token `humpyard ui` printed at start, which it requires when it holds the admin credential.
 * The token is sent once, in a request body, and exchanged for a cookie the page cannot read; it is not stored.
 */
export function TokenEntry(props: { client: Client; onDone: () => void }) {
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(event: Event) {
    event.preventDefault();
    const token = value.trim();
    if (token === "") return;
    setBusy(true);
    setError(null);
    try {
      await props.client.exchangeToken(token);
      props.onDone();
    } catch (failure) {
      setError(failure instanceof ApiError ? failure.message : "The token was not accepted.");
      setBusy(false);
    }
  }

  return (
    <form class="signin" onSubmit={(event) => void submit(event)}>
      <h1>humpyard admin</h1>
      <label>
        Access token
        <input
          type="password"
          autocomplete="off"
          spellcheck={false}
          value={value}
          onInput={(event) => {
            setValue(event.currentTarget.value);
          }}
        />
      </label>
      <p class="hint">Printed when <code>humpyard ui</code> started. It holds the admin credential, so this page needs no other.</p>
      {error !== null && <p role="alert" class="error">{error}</p>}
      <button type="submit" disabled={busy}>Continue</button>
    </form>
  );
}
