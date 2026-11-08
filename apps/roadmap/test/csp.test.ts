import { expect, test } from "vitest";
import { CSP } from "../src/lib/csp.ts";

test("allows only stylesheets and images of the site itself, and no script", () => {
  expect(CSP).toBe("default-src 'none'; style-src 'self'; img-src 'self'; base-uri 'none'; form-action 'none'");
  expect(CSP).not.toContain("script-src");
  expect(CSP).not.toContain("unsafe");
});
