// Every string from the API is rendered as text (ADR 0032): nothing in the source may insert raw HTML or run
// strings as code. A later change that needs one must change this rule, in review, not slip past it.
export const rawHtml = [
  { selector: "JSXAttribute[name.name='dangerouslySetInnerHTML']", message: "Do not insert raw HTML; render API strings as text." },
  { selector: "MemberExpression[property.name=/^(innerHTML|outerHTML)$/]", message: "Do not insert raw HTML; render API strings as text." },
  { selector: "CallExpression[callee.property.name=/^(insertAdjacentHTML|createContextualFragment|write|writeln)$/]", message: "Do not insert raw HTML; render API strings as text." },
  { selector: "CallExpression[callee.name='eval']", message: "Do not run strings as code." },
  { selector: "NewExpression[callee.name='Function']", message: "Do not run strings as code." },
];
