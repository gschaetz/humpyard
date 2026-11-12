import { describe, expect, it, vi } from "vitest";
import { ApiError, createClient } from "./client";

function respond(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
}

describe("the admin API client", () => {
  it("asks only /admin/v1 on its own origin, with the credential as a bearer header", async () => {
    const fetchImpl = vi.fn<typeof fetch>().mockResolvedValue(respond(200, { id: "gary", scopes: ["read"] }));
    await createClient(() => "secret", fetchImpl).whoami();
    const [url, init] = fetchImpl.mock.calls[0] ?? [];
    expect(url).toBe("/admin/v1/whoami");
    expect((init?.headers as Record<string, string>).Authorization).toBe("Bearer secret");
    expect(init?.cache).toBe("no-store");
  });

  it("sends no Authorization header when it holds no credential", async () => {
    const fetchImpl = vi.fn<typeof fetch>().mockResolvedValue(respond(200, {}));
    await createClient(() => null, fetchImpl).whoami();
    expect(fetchImpl.mock.calls[0]?.[1]?.headers).not.toHaveProperty("Authorization");
  });

  it("passes the window and grouping as query parameters and leaves out what is not set", async () => {
    const fetchImpl = vi.fn<typeof fetch>().mockImplementation(() => Promise.resolve(respond(200, { data: [] })));
    const client = createClient(() => "k", fetchImpl);
    await client.stats({ window: "7d", groupBy: "target" });
    await client.stats();
    expect(fetchImpl.mock.calls[0]?.[0]).toBe("/admin/v1/stats?window=7d&group_by=target");
    expect(fetchImpl.mock.calls[1]?.[0]).toBe("/admin/v1/stats");
  });

  describe("the series call", () => {
    const request = { window: "24h", interval: "15m", groupBy: "target" as const };

    it("asks /admin/v1/stats/series with the credential as before, and leaves out what is not set", async () => {
      const body = { object: "stats_series", data: [] };
      const fetchImpl = vi.fn<typeof fetch>().mockImplementation(() => Promise.resolve(respond(200, body)));
      const client = createClient(() => "secret", fetchImpl);
      await expect(client.series({ ...request, target: "a b&c", limit: 8 })).resolves.toEqual(body);
      await client.series(request);
      const [url, init] = fetchImpl.mock.calls[0] ?? [];
      expect(url).toBe("/admin/v1/stats/series?window=24h&interval=15m&group_by=target&target=a+b%26c&limit=8");
      expect((init?.headers as Record<string, string>).Authorization).toBe("Bearer secret");
      expect(fetchImpl.mock.calls[1]?.[0]).toBe("/admin/v1/stats/series?window=24h&interval=15m&group_by=target");
    });

    it("maps a 404 to null, meaning the gateway cannot draw a series, not to an error", async () => {
      const gone = { error: { code: "not_found", message: "no such route" } };
      const client = createClient(() => "k", vi.fn<typeof fetch>().mockResolvedValue(respond(404, gone)));
      await expect(client.series(request)).resolves.toBeNull();
    });

    it("still reports a refused credential, a busy gateway and an unreachable one as errors", async () => {
      const refused = { error: { code: "admin_scope_required", message: "needs read" } };
      const forbidden = createClient(() => "k", vi.fn<typeof fetch>().mockResolvedValue(respond(403, refused)));
      await expect(forbidden.series(request)).rejects.toMatchObject({ status: 403, code: "admin_scope_required" });
      const busy = { error: { code: "admin_busy", message: "busy" } };
      const loaded = createClient(() => "k", vi.fn<typeof fetch>().mockResolvedValue(respond(503, busy)));
      await expect(loaded.series(request)).rejects.toMatchObject({ status: 503, code: "admin_busy" });
      const offline = createClient(() => "k", vi.fn<typeof fetch>().mockRejectedValue(new TypeError("offline")));
      await expect(offline.series(request)).rejects.toMatchObject({ status: 0 });
    });
  });

  it("reports a refusal in the API's own words, with its status and code", async () => {
    const error = { error: { code: "admin_credential_invalid", message: "Invalid admin credential." } };
    const client = createClient(() => "bad", vi.fn<typeof fetch>().mockResolvedValue(respond(401, error)));
    await expect(client.health()).rejects.toMatchObject({
      status: 401,
      code: "admin_credential_invalid",
      message: "Invalid admin credential.",
    });
  });

  it("reports a refusal that is not the API's error shape by its status", async () => {
    const html = new Response("<html>bad gateway</html>", { status: 502 });
    const client = createClient(() => "k", vi.fn<typeof fetch>().mockResolvedValue(html));
    const failure = await client.health().catch((e: unknown) => e);
    expect(failure).toBeInstanceOf(ApiError);
    expect(failure).toMatchObject({ status: 502, code: null, message: "The admin API answered 502." });
  });

  it("reports an unreachable gateway with status 0", async () => {
    const client = createClient(() => "k", vi.fn<typeof fetch>().mockRejectedValue(new TypeError("offline")));
    await expect(client.stats()).rejects.toMatchObject({ status: 0 });
  });

  it("reads the contract version from the published document", async () => {
    const document = { openapi: "3.0.3", info: { title: "humpyard admin API", version: "1" } };
    const client = createClient(() => "k", vi.fn<typeof fetch>().mockResolvedValue(respond(200, document)));
    await expect(client.contractVersion()).resolves.toBe("1");
  });

  it("refuses a contract document that has no version", async () => {
    const client = createClient(() => "k", vi.fn<typeof fetch>().mockResolvedValue(respond(200, { info: {} })));
    await expect(client.contractVersion()).rejects.toBeInstanceOf(ApiError);
  });

  it("sends the access token in a request body to /ui/session and never in a URL", async () => {
    const fetchImpl = vi.fn<typeof fetch>().mockResolvedValue(new Response(null, { status: 204 }));
    await createClient(() => null, fetchImpl).exchangeToken("tok");
    const [url, init] = fetchImpl.mock.calls[0] ?? [];
    expect(url).toBe("/ui/session");
    expect(init?.method).toBe("POST");
    expect(init?.body).toBe('{"token":"tok"}');
  });

  it("reports a refused token with the service's code", async () => {
    const error = { error: { code: "ui_token_invalid", message: "That is not the access token." } };
    const client = createClient(() => null, vi.fn<typeof fetch>().mockResolvedValue(respond(401, error)));
    await expect(client.exchangeToken("x")).rejects.toMatchObject({ status: 401, code: "ui_token_invalid" });
  });
});
