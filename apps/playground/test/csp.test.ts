import { readFileSync } from "node:fs";
import { join } from "node:path";
import { expect, test } from "vitest";
import { CHARSET, CSP, CSP_META, withPolicy } from "../csp.ts";

test("allows only the site itself, and compiling WebAssembly", () => {
  expect(CSP).toBe("default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'");
  expect(CSP).not.toContain("unsafe-inline");
  expect(CSP).not.toContain("'unsafe-eval'");
  expect(CSP_META).toBe(`<meta http-equiv="Content-Security-Policy" content="${CSP}" />`);
});

test("declares the policy right after the charset, first in the head", () => {
  expect(CHARSET).toBe('<meta charset="utf-8" />');
  expect(withPolicy(`<head>\n    ${CHARSET}\n    <title>t</title>`)).toBe(`<head>\n    ${CHARSET}\n    ${CSP_META}\n    <title>t</title>`);
});

test("refuses a page without the charset, rather than building it without a policy", () => {
  expect(() => withPolicy('<head><meta charset="UTF-8"></head>')).toThrow(new Error(`no ${CHARSET} in the page: cannot declare its Content Security Policy after it`));
});

test("declares the policy once in the playground page", () => {
  // From the package directory: under jsdom, module URLs are not file URLs.
  const page = withPolicy(readFileSync(join(process.cwd(), "index.html"), "utf8"));
  expect(page.split(CSP_META)).toHaveLength(2);
});
