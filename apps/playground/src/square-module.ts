/** Loading the page's module, which renders the square shown with the sound. */

import { PageModule } from "@poyetik/web-page/page";
import { fetchOk } from "./fetch.ts";
import { SQUARE } from "./settings.ts";

/**
 * Fetches the page's module from `url`, then instantiates it with {@link SQUARE} set up.
 * @throws {Error} `cannot load the square from <url> (<cause>)` when the download fails, or
 * the module's own error when it is not the page's module or refuses the square.
 */
export async function loadSquare(url: string): Promise<PageModule> {
  const response = await fetchOk("the square", url, {});
  return PageModule.instantiate(await response.arrayBuffer(), SQUARE);
}

/** `load`, run once: every call after the first returns the first's promise. */
export function once<T>(load: () => Promise<T>): () => Promise<T> {
  let loading: Promise<T> | undefined;
  return () => {
    loading ??= load();
    return loading;
  };
}
