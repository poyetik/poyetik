/** Loading the page module, which renders the square shown with the sound. */

import { PageModule, PageModuleError } from "@poyetik/web-page/page";
import { fetchOk } from "./fetch.ts";
import { SQUARE } from "./settings.ts";

/**
 * Where loading the square failed, by whose remedy it takes: its `download` (offline, or the
 * server's answer); its `module` that this browser cannot run (no WebAssembly, or bytes it
 * cannot compile); or a `defect` of the page's own module, the same in every browser (not
 * the page module of this interface, a call that traps, the square refused).
 */
export type LoadStage = "download" | "module" | "defect";

/** What a {@link SquareLoadError} says at each stage. */
const SAID: Readonly<Record<LoadStage, string>> = {
  download: "the square's module did not download",
  module: "the square's module cannot run here",
  defect: "the square's module failed to set up",
};

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
    super(SAID[stage], { cause });
    this.name = "SquareLoadError";
    this.stage = stage;
  }
}

/** What loading the square needs of the browser: its `WebAssembly` namespace, missing where it has none (`globalThis` in a page). */
export interface SquareEnvironment {
  /** The `WebAssembly` namespace, if any. */
  readonly WebAssembly?: unknown;
}

/**
 * Fetches the page module from `url`, then instantiates it with {@link SQUARE} set up.
 * @throws {SquareLoadError} At its `download`, caused by `cannot load the square from <url>
 * (<cause>)` when the request fails, the server does not answer 2xx or the body cannot be
 * read (the request's or the body's error kept as its `cause`); at its `module`, caused by `this browser has no WebAssembly` before any request, or
 * by the page module's error whose cause is the browser's `CompileError`; else as a
 * `defect`, caused by the module's own error (another interface, a trap, the square refused).
 */
export async function loadSquare(url: string, environment: SquareEnvironment = globalThis): Promise<PageModule> {
  if (typeof environment.WebAssembly !== "object") {
    throw new SquareLoadError("module", new Error("this browser has no WebAssembly"));
  }
  const response = await fetchOk("the square", url, {}).catch((error: unknown) => {
    throw new SquareLoadError("download", error);
  });
  const bytes = await response.arrayBuffer().catch((error: unknown) => {
    throw new SquareLoadError("download", new Error(`cannot load the square from ${url} (${String(error)})`, { cause: error }));
  });
  return PageModule.instantiate(bytes, SQUARE).catch((error: unknown) => {
    const cannotCompile = error instanceof PageModuleError && error.cause instanceof WebAssembly.CompileError;
    throw new SquareLoadError(cannotCompile ? "module" : "defect", error);
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
      // No other load starts while this one is kept, so its failure clears only itself; the
      // reason is not dropped: every caller holds the rejected promise itself.
      started.catch((_heldByCallers: unknown) => {
        loading = undefined;
      });
    }
    return loading;
  };
}
