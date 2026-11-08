/**
 * The Content Security Policy of the roadmap page: a static page with no script at all,
 * its stylesheets from the site itself. `frame-ancestors` cannot be set in a `<meta>`
 * policy: the host sets it in a header.
 */
export const CSP = ["default-src 'none'", "style-src 'self'", "img-src 'self'", "base-uri 'none'", "form-action 'none'"].join("; ");
