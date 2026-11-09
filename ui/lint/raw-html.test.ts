import tseslint from "typescript-eslint";
import { Linter } from "eslint";
import { describe, expect, it } from "vitest";
import { rawHtml } from "./raw-html.js";

// The rule list is the one eslint.config.js applies; this proves it catches each way of inserting raw HTML.
function lint(code: string) {
  const linter = new Linter();
  return linter.verify(code, [
    {
      files: ["**/*.tsx"],
      languageOptions: { parser: tseslint.parser, parserOptions: { ecmaFeatures: { jsx: true } } },
      rules: { "no-restricted-syntax": ["error", ...rawHtml] },
    },
  ], "x.tsx");
}

describe("the raw-HTML ban", () => {
  it.each([
    ["dangerouslySetInnerHTML", "const a = <div dangerouslySetInnerHTML={{ __html: x }} />;"],
    ["innerHTML", "el.innerHTML = x;"],
    ["outerHTML", "el.outerHTML = x;"],
    ["insertAdjacentHTML", "el.insertAdjacentHTML('beforeend', x);"],
    ["createContextualFragment", "range.createContextualFragment(x);"],
    ["document.write", "document.write(x);"],
    ["eval", "eval(x);"],
    ["Function", "new Function(x);"],
  ])("rejects %s", (_name, code) => {
    expect(lint(code)).toHaveLength(1);
  });

  it("allows rendering a string as text", () => {
    expect(lint("const a = <div>{name}</div>; el.textContent = name;")).toHaveLength(0);
  });
});
