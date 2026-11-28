import { describe, expect, it } from "vitest";
import guide from "../../docs/clients.md?raw";
import { clientSettings, PLACEHOLDER_KEY, pickModels } from "./clients";

const settings = clientSettings("http://127.0.0.1:8080/", ["deep", "fast"]);
const text = (id: string) => settings.find((s) => s.id === id)?.text ?? "";

describe("the settings to copy", () => {
  it("name the live address, without a doubled slash, and the routes the gateway has", () => {
    const claude = text("claude-code");
    expect(claude).toContain("ANTHROPIC_BASE_URL=http://127.0.0.1:8080\n");
    expect(claude).toContain("ANTHROPIC_MODEL=deep");
    expect(claude).toContain("ANTHROPIC_DEFAULT_HAIKU_MODEL=fast");
    expect(text("codex")).toContain('base_url = "http://127.0.0.1:8080/v1"');
    expect(text("openai")).toContain("Base URL: http://127.0.0.1:8080/v1");
  });

  it("hold no secret, only the placeholder", () => {
    for (const s of settings) {
      expect(s.text).not.toMatch(/sk-[A-Za-z0-9-]{8,}/);
    }
    expect(text("claude-code")).toContain(`ANTHROPIC_API_KEY=${PLACEHOLDER_KEY}`);
  });

  it("use the variables and keys that docs/clients.md documents", () => {
    for (const name of text("claude-code").match(/ANTHROPIC_[A-Z_]+/g) ?? []) {
      expect(guide, `${name} is in docs/clients.md`).toContain(name);
    }
    for (const line of ['model_provider = "humpyard"', 'env_key = "HUMPYARD_KEY"', 'wire_api = "responses"', "[model_providers.humpyard]"]) {
      expect(guide, `${line} is in docs/clients.md`).toContain(line);
    }
  });

  it("choose models from the routes there are", () => {
    expect(pickModels(["deep", "fast"])).toEqual({ main: "deep", small: "fast" });
    expect(pickModels(["only"])).toEqual({ main: "only", small: "only" });
    expect(pickModels([])).toEqual({ main: "fast", small: "fast" });
  });
});
