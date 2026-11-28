// `humpyard up` opens the page on `#token=<access token>`, so the person never pastes anything. The fragment never
// reaches a server or a Referer, and the token is taken out of the address before it is used, so it is not in the
// history entry, a bookmark or a screenshot afterwards. Anything else in the fragment (the query view's) stays.

const NAME = "token";

/** The token in a fragment, and the fragment without it. A fragment with no token comes back unchanged. */
export function takeToken(hash: string): { token: string | null; rest: string } {
  const body = hash.startsWith("#") ? hash.slice(1) : hash;
  const params = new URLSearchParams(body);
  const token = params.get(NAME);
  if (token === null) return { token: null, rest: hash };
  params.delete(NAME);
  const rest = params.toString();
  return { token: token === "" ? null : token, rest: rest === "" ? "" : `#${rest}` };
}

/** Removes the token from the address bar without adding a history entry; returns the token that was there. */
export function takeTokenFromLocation(): string | null {
  const { token, rest } = takeToken(window.location.hash);
  if (window.location.hash !== rest && window.location.hash.includes(`${NAME}=`)) {
    window.history.replaceState(null, "", `${window.location.pathname}${window.location.search}${rest}`);
  }
  return token;
}
