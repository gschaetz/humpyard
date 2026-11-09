import type { components, paths } from "./schema";

export type Stats = paths["/admin/v1/stats"]["get"]["responses"][200]["content"]["application/json"];
export type StatsGroup = components["schemas"]["StatsGroup"];
export type Health = components["schemas"]["Health"];
export type EndpointHealth = components["schemas"]["EndpointHealth"];
export type Principal = components["schemas"]["Principal"];

/** Why a call to the admin API did not produce data. `status` is 0 when the gateway could not be reached at all. */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: string | null,
    message: string,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

export interface StatsQuery {
  /** How far back, for example `1h` or `7d`. */
  window?: string;
  groupBy?: "route" | "target" | "endpoint";
}

export interface Client {
  whoami(): Promise<Principal>;
  /** `info.version` of the admin API's published contract. */
  contractVersion(): Promise<string>;
  stats(query?: StatsQuery): Promise<Stats>;
  health(): Promise<Health>;
}

const PREFIX = "/admin/v1/";

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
  async function getJson<T>(path: string, query?: Record<string, string | undefined>): Promise<T> {
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
      response = await fetchImpl(`${path}${search}`, { headers, cache: "no-store" });
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
    stats: (query = {}) => getJson<Stats>("/admin/v1/stats", { window: query.window, group_by: query.groupBy }),
    health: () => getJson<Health>("/admin/v1/health"),
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
  return new ApiError(response.status, code, message);
}
