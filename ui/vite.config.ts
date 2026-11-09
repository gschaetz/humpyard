import preact from "@preact/preset-vite";
import { defineConfig } from "vitest/config";

// The page is served under a strict Content-Security-Policy with no `unsafe-inline` (ADR 0032), so the build must
// emit external scripts and styles only: nothing is inlined and the preload polyfill (which is script text) is off.
export default defineConfig({
  plugins: [preact()],
  build: {
    assetsInlineLimit: 0,
    cssCodeSplit: false,
    modulePreload: { polyfill: false },
    sourcemap: false,
  },
  test: {
    environment: "jsdom",
    include: ["src/**/*.test.{ts,tsx}", "lint/*.test.ts"],
  },
});
