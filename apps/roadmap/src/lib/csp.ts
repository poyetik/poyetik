/**
 * The Content Security Policy of the roadmap page: a static page with no script at all,
 * its stylesheets from the site itself. `frame-ancestors` cannot be set in a `<meta>`
 * policy: the host sets it in a header.
 */
export const CSP = ["default-src 'none'", "style-src 'self'", "img-src 'self'", "base-uri 'none'", "form-action 'none'"].join("; ");

/**
 * The policy the page declares in a given Vite mode (`import.meta.env.MODE`).
 *
 * Only the dev server (`astro dev`, mode `development`) gets no policy. It injects the page's
 * styles inline (`<style>`) and loads its hot-reload client as a script, which the strict policy
 * blocks: the page would show without any style. Relaxing the policy there only affects local
 * development; it is never shipped. Every other mode, the build (`production`) and the tests
 * (`test`) included, keeps the strict policy, so a test always checks what is published.
 */
export function pagePolicy(mode: string): string | undefined {
  return mode === "development" ? undefined : CSP;
}
