/// <reference types="astro/client" />

// Type for .astro component imports outside the Astro compiler (tsc, ESLint, tests).
declare module "*.astro" {
  import type { experimental_AstroContainer } from "astro/container";

  const component: Parameters<experimental_AstroContainer["renderToString"]>[0];
  export default component;
}
