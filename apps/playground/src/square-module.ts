/** Loading the page module, which renders the square shown with the sound. */

import { PageModule } from "@poyetik/web-page/page";
import { fetchBytes } from "./fetch.ts";
import { SQUARE } from "./settings.ts";
import { cannotCompile, isWasmModule } from "./wasm-bytes.ts";

/**
 * Where loading the square failed, by whose remedy it takes: its `download` (offline, the
 * server's answer, or bytes that are no WebAssembly module, such as a page served in its
 * place); its `module` that this browser cannot run (no WebAssembly, or a module it cannot
 * compile); or a `defect` of the page's own module, the same in every browser (not the page
 * module of this interface, a call that traps, the square refused).
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
 * How long the square's download may take, in milliseconds, before it is aborted and fails:
 * the page module, about 70 kB, takes about 11 s over a link of 50 kbit/s, as slow as a
 * mobile network still serves a page; 30 s leaves more than twice that, so that only a stall
 * (a body that never comes) reaches it, and the page says so instead of loading for good.
 */
export const SQUARE_DOWNLOAD_MS = 30_000;

/**
 * Fetches the page module from `url`, then instantiates it with {@link SQUARE} set up.
 * @throws {SquareLoadError} At its `download`, caused by `cannot load the square from <url>`
 * when the request fails, the body cannot be read, or the download outlasts
 * {@link SQUARE_DOWNLOAD_MS} (the request's or the body's error kept as its `cause`, aborted
 * by `no answer within 30 s`), by `cannot load the square from <url> (HTTP <status>)` when the server
 * does not answer 2xx, or by `cannot load the
 * square from <url> (not a WebAssembly module)` when the bytes do not start as one
 * (`isWasmModule`); at its `module`, caused by `this browser has no WebAssembly` before any
 * request, or by the page module's error whose cause is the browser's `CompileError`
 * (`cannotCompile`); else as a `defect`, caused by the module's own error (another
 * interface, a trap, the square refused).
 */
export async function loadSquare(url: string, environment: SquareEnvironment = globalThis): Promise<PageModule> {
  if (typeof environment.WebAssembly !== "object") {
    throw new SquareLoadError("module", new Error("this browser has no WebAssembly"));
  }
  const download = new AbortController();
  const bound = setTimeout(() => {
    download.abort(new Error(`no answer within ${String(SQUARE_DOWNLOAD_MS / 1000)} s`));
  }, SQUARE_DOWNLOAD_MS);
  const bytes = await fetchBytes("the square", url, { signal: download.signal })
    .catch((error: unknown) => {
      throw new SquareLoadError("download", error);
    })
    .finally(() => {
      clearTimeout(bound);
    });
  if (!isWasmModule(bytes)) {
    throw new SquareLoadError("download", new Error(`cannot load the square from ${url} (not a WebAssembly module)`));
  }
  return PageModule.instantiate(bytes, SQUARE).catch((error: unknown) => {
    throw new SquareLoadError(cannotCompile(error) ? "module" : "defect", error);
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
