// The built page is served under a Content-Security-Policy with no `unsafe-inline` (ADR 0032), so it may contain no
// inline script, inline style or inline event handler. Run after `npm run build`.
import { readFileSync } from "node:fs";

const html = readFileSync(new URL("../dist/index.html", import.meta.url), "utf8");
const problems = [];
for (const match of html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/gi)) {
  if (!/\bsrc=/.test(match[1]) || match[2].trim() !== "") problems.push(`inline script: ${match[0].slice(0, 60)}`);
}
if (/<style\b/i.test(html)) problems.push("inline <style> element");
if (/\sstyle=/i.test(html)) problems.push("inline style attribute");
if (/\son[a-z]+=/i.test(html)) problems.push("inline event handler");
if (problems.length > 0) {
  console.error(`dist/index.html is not CSP-clean:\n- ${problems.join("\n- ")}`);
  process.exit(1);
}
console.log("dist/index.html has no inline script or style");
