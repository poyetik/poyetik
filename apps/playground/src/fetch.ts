// Fetching what the playground needs: the one place that calls `fetch` (the shared lint
// policy refuses it elsewhere), so that no response is read before its status is checked, and
// none leaves it: its body is read here, its failure named as the request's.

/**
 * The bytes at `url`, described as `what` in errors, fetched with `init`, which holds an
 * abort signal by type: no download of the page is left without a bound or a cancel.
 * @throws {Error} `cannot load <what> from <url>` when the request or the reading of its body
 * fails, the error of `fetch` or of the body kept as its `cause` (`failure` words it);
 * `cannot load <what> from <url> (HTTP <status>)`, without a `cause`, when the server does not
 * answer 2xx.
 */
export async function fetchBytes(what: string, url: string, init: RequestInit & { readonly signal: AbortSignal }): Promise<ArrayBuffer> {
  /** The error naming what failed to load, `error` its cause. */
  const failed = (error: unknown): never => {
    throw new Error(`cannot load ${what} from ${url}`, { cause: error });
  };
  const response = await fetch(url, init).catch(failed);
  if (!response.ok) {
    throw new Error(`cannot load ${what} from ${url} (HTTP ${String(response.status)})`);
  }
  return response.arrayBuffer().catch(failed);
}
