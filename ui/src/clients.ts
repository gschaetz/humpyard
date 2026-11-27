/**
 * The settings a person copies to point a tool at this gateway, built from its live address and its route names. They
 * hold no secret: on this machine the gateway has no client keys, so a tool only needs some value where it asks for
 * one. `src/clients.test.ts` checks each against `docs/clients.md`, so the page and the guide cannot drift apart.
 */
export interface ClientSettings {
  id: string;
  title: string;
  /** Where the text goes, shown above it. */
  where: string;
  text: string;
}

/** The value put where a tool insists on a key. Any value works on this machine. */
export const PLACEHOLDER_KEY = "humpyard";

/** The model a tool should ask for by default and the one for small, cheap calls, from the routes the gateway has. */
export function pickModels(routes: string[]): { main: string; small: string } {
  const main = routes.includes("deep") ? "deep" : (routes[0] ?? "fast");
  const small = routes.includes("fast") ? "fast" : main;
  return { main, small };
}

export function clientSettings(gateway: string, routes: string[]): ClientSettings[] {
  const { main, small } = pickModels(routes);
  const base = gateway.replace(/\/+$/, "");
  return [
    {
      id: "claude-code",
      title: "Claude Code",
      where: "Run in the terminal where you start Claude Code",
      text: [
        `export ANTHROPIC_BASE_URL=${base}`,
        `export ANTHROPIC_API_KEY=${PLACEHOLDER_KEY}`,
        `export ANTHROPIC_MODEL=${main}`,
        `export ANTHROPIC_DEFAULT_SONNET_MODEL=${main}`,
        `export ANTHROPIC_DEFAULT_OPUS_MODEL=${main}`,
        `export ANTHROPIC_DEFAULT_HAIKU_MODEL=${small}`,
        `export ANTHROPIC_SMALL_FAST_MODEL=${small}`,
        "claude",
      ].join("\n"),
    },
    {
      id: "codex",
      title: "Codex CLI",
      where: "Put in ~/.codex/config.toml, and set the key in your shell",
      text: [
        `# export HUMPYARD_KEY=${PLACEHOLDER_KEY}`,
        `model_provider = "humpyard"`,
        `model = "${main}"`,
        "",
        "[model_providers.humpyard]",
        `name = "humpyard"`,
        `base_url = "${base}/v1"`,
        `env_key = "HUMPYARD_KEY"`,
        `wire_api = "responses"`,
      ].join("\n"),
    },
    {
      id: "openai",
      title: "Any OpenAI-compatible tool",
      where: "Enter in the tool's settings",
      text: [`Base URL: ${base}/v1`, `API key: ${PLACEHOLDER_KEY}   (any value works on this machine)`, `Model: ${main}`].join("\n"),
    },
    {
      id: "curl",
      title: "curl",
      where: "Run in a terminal",
      text: [
        `curl ${base}/v1/chat/completions \\`,
        `  -H 'content-type: application/json' \\`,
        `  -d '{"model":"${small}","messages":[{"role":"user","content":"hi"}]}'`,
      ].join("\n"),
    },
  ];
}
