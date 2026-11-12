import type { components, paths } from "./schema";

export type Stats = paths["/admin/v1/stats"]["get"]["responses"][200]["content"]["application/json"];
export type StatsGroup = components["schemas"]["StatsGroup"];
export type Series = paths["/admin/v1/stats/series"]["get"]["responses"][200]["content"]["application/json"];
export type SeriesGroup = components["schemas"]["SeriesGroup"];
export type SeriesBucket = components["schemas"]["SeriesBucket"];
export type Health = components["schemas"]["Health"];
export type EndpointHealth = components["schemas"]["EndpointHealth"];
export type Principal = components["schemas"]["Principal"];

/** Why a call to the admin API did not produce data. `status` is 0 when the gateway could not be reached at all. */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string | null,
    message: string,
    /** The `Retry-After` the gateway sent, in milliseconds; null when it sent none. */
    readonly retryAfterMs: number | null = null,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

export interface StatsQuery {
  /** How far back, for example `1h` or `7d`. */
  window?: string;
  groupBy?: "route" | "target" | "endpoint";
  route?: string;
  target?: string;
}

/** Lets a caller drop a request it no longer wants (the query changed). */
export interface Options {
  signal?: AbortSignal;
}

export interface SeriesRequest {
  /** How far back, for example `24h`. */
  window: string;
  /** The bucket width, for example `15m`. */
  interval: string;
  groupBy: "route" | "target" | "endpoint";
  route?: string;
  target?: string;
  /** The most groups returned (the gateway's own default applies when unset). */
  limit?: number;
}

export interface Client {
  whoami(): Promise<Principal>;
  /** `info.version` of the admin API's published contract. */
  contractVersion(): Promise<string>;
  stats(query?: StatsQuery, options?: Options): Promise<Stats>;
  /**
   * Statistics over time, or `null` when the gateway has no series endpoint (it is older than this page): the
   * caller shows the table without a chart rather than an error.
   */
  series(request: SeriesRequest, options?: Options): Promise<Series | null>;
  health(options?: Options): Promise<Health>;
  /**
   * Exchanges the access token `humpyard ui --credential-env` printed at start for a session cookie (the only call
   * outside `/admin/v1`). The token goes in the body, never in a URL.
   */
  exchangeToken(token: string): Promise<void>;
}

const PREFIX = "/admin/v1/";
const SESSION_PATH = "/ui/session";

/**
 * A client of `/admin/v1` on the origin the page was loaded from, and of nothing else: every path is a constant
 * under the prefix, relative, so there is no way to aim a request at another host. `credential` is read on each
 * call, so signing in or out needs no new client; with none held no `Authorization` header is sent (a
 * `humpyard ui` started with `--credential-env` adds its own).
 */
export function createClient(
  credential: () => string | null,
  fetchImpl: typeof fetch = (input, init) => fetch(input, init),
): Client {
  async function getJson<T>(path: string, query?: Record<string, string | undefined>, options?: Options): Promise<T> {
    if (!path.startsWith(PREFIX)) {
      throw new Error(`refusing to request ${path}: not under ${PREFIX}`);
    }
    const params = new URLSearchParams();
    for (const [name, value] of Object.entries(query ?? {})) {
      if (value !== undefined) params.set(name, value);
    }
    const search = params.size > 0 ? `?${params.toString()}` : "";
    const headers: Record<string, string> = { Accept: "application/json" };
    const held = credential();
    if (held !== null) headers.Authorization = `Bearer ${held}`;

    let response: Response;
    try {
      response = await fetchImpl(`${path}${search}`, { headers, cache: "no-store", signal: options?.signal });
    } catch {
      throw new ApiError(0, null, "The admin API could not be reached.");
    }
    if (!response.ok) throw await refusal(response);
    try {
      return (await response.json()) as T;
    } catch {
      throw new ApiError(response.status, null, "The admin API answered with something that is not JSON.");
    }
  }

  return {
    whoami: () => getJson<Principal>("/admin/v1/whoami"),
    contractVersion: async () => {
      const document = await getJson<{ info?: { version?: unknown } }>("/admin/v1/openapi.json");
      const version = document.info?.version;
      if (typeof version !== "string") {
        throw new ApiError(200, null, "The admin API's contract has no version.");
      }
      return version;
    },
    stats: (query = {}, options) =>
      getJson<Stats>(
        "/admin/v1/stats",
        { window: query.window, group_by: query.groupBy, route: query.route, target: query.target },
        options,
      ),
    series: async (request, options) => {
      try {
        return await getJson<Series>(
          "/admin/v1/stats/series",
          {
            window: request.window,
            interval: request.interval,
            group_by: request.groupBy,
            route: request.route,
            target: request.target,
            limit: request.limit?.toString(),
          },
          options,
        );
      } catch (failure) {
        if (failure instanceof ApiError && failure.status === 404) return null;
        throw failure;
      }
    },
    health: (options) => getJson<Health>("/admin/v1/health", undefined, options),
    exchangeToken: async (token) => {
      let response: Response;
      try {
        response = await fetchImpl(SESSION_PATH, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ token }),
          cache: "no-store",
        });
      } catch {
        throw new ApiError(0, null, "The service could not be reached.");
      }
      if (!response.ok) throw await refusal(response);
    },
  };
}

async function refusal(response: Response): Promise<ApiError> {
  let code: string | null = null;
  let message = `The admin API answered ${response.status.toString()}.`;
  try {
    const body = (await response.json()) as { error?: { code?: unknown; message?: unknown } };
    if (typeof body.error?.code === "string") code = body.error.code;
    if (typeof body.error?.message === "string") message = body.error.message;
  } catch {
    // Not the admin API's error shape: the status line above is the message.
  }
  const seconds = Number(response.headers.get("Retry-After"));
  const retryAfterMs = Number.isFinite(seconds) && seconds > 0 ? seconds * 1000 : null;
  return new ApiError(response.status, code, message, retryAfterMs);
}
