import { useState } from "preact/hooks";
import type { Client } from "./api/client";
import { ApiError } from "./api/client";
import { saveCredential } from "./api/session";

/**
 * Asks for an admin credential. It is checked against `/admin/v1/whoami` and kept for the browser session only; a
 * wrong one is reported in the API's own words and is not kept.
 */
export function SignIn(props: { client: Client; hold: (credential: string) => void; onSignedIn: () => void }) {
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(event: Event) {
    event.preventDefault();
    const credential = value.trim();
    if (credential === "") return;
    setBusy(true);
    setError(null);
    props.hold(credential);
    try {
      await props.client.whoami();
      saveCredential(credential);
      props.onSignedIn();
    } catch (failure) {
      props.hold("");
      setError(failure instanceof ApiError ? failure.message : "Sign-in failed.");
      setBusy(false);
    }
  }

  return (
    <form class="signin" onSubmit={(event) => void submit(event)}>
      <h1>humpyard admin</h1>
      <label>
        Admin credential
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
      <p class="hint">Kept for this browser session only.</p>
      {error !== null && <p role="alert" class="error">{error}</p>}
      <button type="submit" disabled={busy}>Sign in</button>
    </form>
  );
}
