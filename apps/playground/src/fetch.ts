// Fetching what the playground needs: the one place that calls `fetch` (the shared lint
// policy refuses it elsewhere), so that no response is read before its status is checked.

/**
 * Fetches `url`, described as `what` in errors, with `init` (an abort signal, for instance).
 * @throws {Error} `cannot load <what> from <url> (<cause>)`, when the request fails (the
 * cause is the error of `fetch`) or the server does not answer 2xx (the cause is
 * `HTTP <status>`).
 */
export async function fetchOk(what: string, url: string, init: RequestInit): Promise<Response> {
  const response = await fetch(url, init).catch((error: unknown) => {
    throw new Error(`cannot load ${what} from ${url} (${String(error)})`);
  });
  if (!response.ok) {
    throw new Error(`cannot load ${what} from ${url} (HTTP ${String(response.status)})`);
  }
  return response;
}
