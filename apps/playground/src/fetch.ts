// Fetching what the playground needs: the one place that calls `fetch` (the shared lint
// policy refuses it elsewhere), so that no response is read before its status is checked.

/**
 * Fetches `url`, described as `what` in errors, with `init` (an abort signal, for instance).
 * @throws {Error} `cannot load <what> from <url>` when the request fails, the error of
 * `fetch` kept as its `cause` (`failure` words it); `cannot load <what> from <url>
 * (HTTP <status>)`, without a `cause`, when the server does not answer 2xx.
 */
export async function fetchOk(what: string, url: string, init: RequestInit): Promise<Response> {
  const response = await fetch(url, init).catch((error: unknown) => {
    throw new Error(`cannot load ${what} from ${url}`, { cause: error });
  });
  if (!response.ok) {
    throw new Error(`cannot load ${what} from ${url} (HTTP ${String(response.status)})`);
  }
  return response;
}
