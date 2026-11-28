import { ApiError } from "./client";

/**
 * The calls only `humpyard up` answers, all under `/local` on the origin the page came from (ADR 0039). Under
 * `humpyard ui` none of them exists, and `connect` says so by returning `null`. No call returns a key and none of
 * them can be asked to: a key goes out in `setKey` and is never read back.
 */
export interface ConnectInfo {
  /** The gateway's own address, like `http://127.0.0.1:8080`. */
  gateway: string;
  /** The model names a client may ask for. */
  routes: string[];
  has_config: boolean;
  gateway_running: boolean;
  /** False when the page is served beyond this machine: keys cannot be entered. */
  key_entry: boolean;
}

export type KeySource = "environment" | "file" | "session" | "none";

export interface KeyStatus {
  provider: string;
  variable: string;
  source: KeySource;
}

export interface TestResult {
  ok: boolean;
  model?: string;
  target?: string;
  latency_ms?: number;
  step?: string;
  message?: string;
}

export interface LocalClient {
  /** The facts for the connect panel, or `null` when this page is not served by `humpyard up`. */
  connect(): Promise<ConnectInfo | null>;
  keys(): Promise<{ key_entry: boolean; providers: KeyStatus[] }>;
  /** Sends a key. `save` keeps it on this machine; otherwise it is held for this run only. Returns the service's note. */
  setKey(provider: string, key: string, save: boolean): Promise<string>;
  removeKey(provider: string): Promise<string>;
  test(): Promise<TestResult>;
}

const BASE = "/local/";

export function createLocalClient(fetchImpl: typeof fetch = (input, init) => fetch(input, init)): LocalClient {
  async function call(method: string, path: string, body?: unknown): Promise<Response> {
    if (!path.startsWith(BASE)) throw new Error(`refusing to request ${path}: not under ${BASE}`);
    const headers: Record<string, string> = { Accept: "application/json" };
    if (body !== undefined) headers["Content-Type"] = "application/json";
    try {
      return await fetchImpl(path, {
        method,
        headers,
        cache: "no-store",
        body: body === undefined ? undefined : JSON.stringify(body),
      });
    } catch {
      throw new ApiError(0, null, "The service could not be reached.");
    }
  }

  async function json<T>(response: Response): Promise<T> {
    if (!response.ok) {
      let code: string | null = null;
      let message = `The service answered ${response.status.toString()}.`;
      try {
        const parsed = (await response.json()) as { error?: { code?: unknown; message?: unknown } };
        if (typeof parsed.error?.code === "string") code = parsed.error.code;
        if (typeof parsed.error?.message === "string") message = parsed.error.message;
      } catch {
        // The words above stand.
      }
      throw new ApiError(response.status, code, message);
    }
    return (await response.json()) as T;
  }

  return {
    connect: async () => {
      let response: Response;
      try {
        response = await call("GET", "/local/connect");
      } catch {
        return null;
      }
      // Anything but a JSON answer (the page itself, a 404, a 401) means this is not a service with `/local`.
      if (!response.ok || !(response.headers.get("content-type") ?? "").startsWith("application/json")) return null;
      return (await response.json()) as ConnectInfo;
    },
    keys: async () => json(await call("GET", "/local/keys")),
    setKey: async (provider, key, save) => {
      const answer = await json<{ note?: string }>(await call("POST", "/local/keys", { provider, key, save }));
      return answer.note ?? "";
    },
    removeKey: async (provider) => {
      const answer = await json<{ note?: string }>(await call("DELETE", `/local/keys/${encodeURIComponent(provider)}`, {}));
      return answer.note ?? "";
    },
    test: async () => json(await call("POST", "/local/test", {})),
  };
}
