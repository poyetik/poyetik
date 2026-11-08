import { expect, test } from "vitest";
import { CSP, CSP_META } from "../csp.ts";

test("allows only the site itself, and compiling WebAssembly", () => {
  expect(CSP).toBe("default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; connect-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'");
  expect(CSP).not.toContain("unsafe-inline");
  expect(CSP).not.toContain("'unsafe-eval'");
  expect(CSP_META).toBe(`<meta http-equiv="Content-Security-Policy" content="${CSP}" />`);
});
