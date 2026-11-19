/** Loading the page module, which renders the square shown with the sound. */

import { PageModule } from "@poyetik/web-page/page";
import { fetchOk } from "./fetch.ts";
import { SQUARE } from "./settings.ts";

/**
 * Fetches the page module from `url`, then instantiates it with {@link SQUARE} set up.
 * @throws {Error} `cannot load the square from <url> (<cause>)` when the download fails, or
 * the module's own error when it is not the page module or refuses the square.
 */
export async function loadSquare(url: string): Promise<PageModule> {
  const response = await fetchOk("the square", url, {});
  return PageModule.instantiate(await response.arrayBuffer(), SQUARE);
}

/**
 * `load`, run once while it succeeds: every call after the first returns the first's
 * promise, but a load that fails is forgotten, so that the next call loads again.
 */
export function once<T>(load: () => Promise<T>): () => Promise<T> {
  let loading: Promise<T> | undefined;
  return () => {
    if (loading === undefined) {
      const started = load();
      loading = started;
      // No other load starts while this one is kept, so its failure clears only itself.
      started.catch(() => {
        loading = undefined;
      });
    }
    return loading;
  };
}
