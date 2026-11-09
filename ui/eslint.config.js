import js from "@eslint/js";
import tseslint from "typescript-eslint";
import { rawHtml } from "./lint/raw-html.js";

export default tseslint.config(
  { ignores: ["dist", "node_modules", "src/api/schema.d.ts", "eslint.config.js", "lint", "scripts"] },
  js.configs.recommended,
  ...tseslint.configs.strictTypeChecked,
  {
    languageOptions: { parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname } },
    rules: { "no-restricted-syntax": ["error", ...rawHtml] },
  },
);
