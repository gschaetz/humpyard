// The admin credential lives for the browser session only (ADR 0032): `sessionStorage`, never `localStorage`, a
// cookie or anything that outlives the tab. Where storage is blocked it is kept in memory, which is shorter still.

const KEY = "humpyard.admin.credential";
let memory: string | null = null;

export function loadCredential(): string | null {
  try {
    return sessionStorage.getItem(KEY) ?? memory;
  } catch {
    return memory;
  }
}

export function saveCredential(credential: string): void {
  memory = credential;
  try {
    sessionStorage.setItem(KEY, credential);
  } catch {
    // Blocked storage: the in-memory copy above is all there is.
  }
}

export function clearCredential(): void {
  memory = null;
  try {
    sessionStorage.removeItem(KEY);
  } catch {
    // Nothing was stored.
  }
}
