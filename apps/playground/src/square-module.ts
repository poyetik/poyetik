/** Loading the page module, which renders the square shown with the sound. */

import { PageModule } from "@poyetik/web-page/page";
import { fetchOk } from "./fetch.ts";
import { SQUARE } from "./settings.ts";

/**
 * Where loading the square failed: its `download` (offline, or the server's answer), or its
 * `module` (not the page module, refused by this browser, which may lack WebAssembly, or
 * refusing the square), each with its own remedy.
 */
export type LoadStage = "download" | "module";

/** Loading the square failed at {@link LoadStage}: its cause is what failed. */
export class SquareLoadError extends Error {
  /** Where it failed. */
  readonly stage: LoadStage;

  /**
   * A failure at `stage`, caused by `cause`.
   * @param stage Where it failed.
   * @param cause What failed.
   */
  constructor(stage: LoadStage, cause: unknown) {
    super(stage === "download" ? "the square's module did not download" : "the square's module cannot run here", { cause });
    this.stage = stage;
  }
}

/**
 * Fetches the page module from `url`, then instantiates it with {@link SQUARE} set up.
 * @throws {SquareLoadError} At its `download`, caused by `cannot load the square from
 * <url> (<cause>)`; at its `module`, caused by the module's own error when it is not the
 * page module, cannot run in this browser or refuses the square.
 */
export async function loadSquare(url: string): Promise<PageModule> {
  const bytes = await fetchOk("the square", url, {})
    .then((response) => response.arrayBuffer())
    .catch((error: unknown) => {
      throw new SquareLoadError("download", error);
    });
  return PageModule.instantiate(bytes, SQUARE).catch((error: unknown) => {
    throw new SquareLoadError("module", error);
  });
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
