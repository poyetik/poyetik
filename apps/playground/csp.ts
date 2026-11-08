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
