import { describe, expect, it, vi } from "vitest";
import { ApiError } from "./client";
import { createLocalClient } from "./local";

function respond(status: number, body: unknown, type = "application/json"): Response {
  return new Response(typeof body === "string" ? body : JSON.stringify(body), { status, headers: { "content-type": type } });
}

describe("the local client", () => {
  it("says it is not served by humpyard up when connect is not JSON, missing or refused", async () => {
    for (const answer of [respond(200, "<html></html>", "text/html"), respond(404, { error: {} }), respond(401, { error: {} })]) {
      expect(await createLocalClient(() => Promise.resolve(answer)).connect()).toBeNull();
    }
    expect(await createLocalClient(() => Promise.reject(new Error("down"))).connect()).toBeNull();
  });

  it("reads the connect facts", async () => {
    const info = { gateway: "http://127.0.0.1:8080", routes: ["fast"], has_config: true, gateway_running: true, key_entry: true };
    expect(await createLocalClient(() => Promise.resolve(respond(200, info))).connect()).toEqual(info);
  });

  it("sends a key in a JSON body to /local/keys and never in a URL", async () => {
    const fetchImpl = vi.fn<typeof fetch>().mockResolvedValue(respond(200, { ok: true, note: "held for this run only" }));
    const note = await createLocalClient(fetchImpl).setKey("groq", "gsk-secret", false);
    expect(note).toBe("held for this run only");
    const [url, init] = fetchImpl.mock.calls[0] ?? [];
    expect(url).toBe("/local/keys");
    expect(url).not.toContain("gsk-secret");
    expect(init?.method).toBe("POST");
    const body = init?.body;
    expect(typeof body === "string" ? (JSON.parse(body) as unknown) : null).toEqual({ provider: "groq", key: "gsk-secret", save: false });
    expect((init?.headers as Record<string, string>)["Content-Type"]).toBe("application/json");
  });

  it("removes a key by provider name, encoded", async () => {
    const fetchImpl = vi.fn<typeof fetch>().mockResolvedValue(respond(200, { ok: true, note: "removed" }));
    await createLocalClient(fetchImpl).removeKey("a/b");
    expect(fetchImpl.mock.calls[0]?.[0]).toBe("/local/keys/a%2Fb");
    expect(fetchImpl.mock.calls[0]?.[1]?.method).toBe("DELETE");
  });

  it("reports a refusal in the service's words, without the request", async () => {
    const refusal = respond(422, { error: { code: "provider_rejected_key", message: "groq did not accept this key: 401" } });
    const failure = await createLocalClient(() => Promise.resolve(refusal))
      .setKey("groq", "gsk-secret", true)
      .catch((e: unknown) => e);
    expect(failure).toBeInstanceOf(ApiError);
    expect((failure as ApiError).code).toBe("provider_rejected_key");
    expect((failure as ApiError).message).toBe("groq did not accept this key: 401");
    expect(JSON.stringify(failure)).not.toContain("gsk-secret");
  });
});
