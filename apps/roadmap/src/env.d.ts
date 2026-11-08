/// <reference types="astro/client" />

/** The type of an `.astro` component import outside the Astro compiler (tsc, ESLint, tests). */
declare module "*.astro" {
  import type { experimental_AstroContainer } from "astro/container";

  /** The component the file defines, in the form the Astro container renders. */
  const component: Parameters<experimental_AstroContainer["renderToString"]>[0];
  export default component;
}
