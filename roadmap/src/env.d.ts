/// <reference types="astro/client" />

// Type for .astro component imports outside the Astro compiler (tsc, ESLint, tests).
declare module "*.astro" {
  const component: Parameters<import("astro/container").experimental_AstroContainer["renderToString"]>[0];
  export default component;
}
