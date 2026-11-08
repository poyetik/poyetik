/**
 * The Content Security Policy of the built playground: everything comes from the site
 * itself, nothing else runs. `'wasm-unsafe-eval'` lets the page and the AudioWorklet
 * compile the engine module (and allows nothing else: no `eval`, no inline script).
 * `frame-ancestors` cannot be set in a `<meta>` policy: the host sets it in a header.
 */
export const CSP = [
  "default-src 'none'",
  "script-src 'self' 'wasm-unsafe-eval'",
  "style-src 'self'",
  "connect-src 'self'",
  "img-src 'self'",
  "base-uri 'none'",
  "form-action 'none'",
].join("; ");

/** The `<meta>` element declaring {@link CSP}, first in the `<head>` of the built page. */
export const CSP_META = `<meta http-equiv="Content-Security-Policy" content="${CSP}" />`;

/** The element of the page the policy follows: the first of its `<head>`, as the policy must come before anything it governs. */
export const CHARSET = '<meta charset="utf-8" />';

/**
 * `page` with {@link CSP_META} right after {@link CHARSET}.
 * @throws {Error} when the page has no such element: a page built without its policy
 * would still work, so the build must fail instead.
 */
export function withPolicy(page: string): string {
  if (!page.includes(CHARSET)) {
    throw new Error(`no ${CHARSET} in the page: cannot declare its Content Security Policy after it`);
  }
  return page.replace(CHARSET, `${CHARSET}\n    ${CSP_META}`);
}
