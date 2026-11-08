// Fetching what the test bench needs: a missing build or case is an error naming the URL
// and the status, never an empty body parsed as JSON or compiled as WebAssembly.

/**
 * Fetches `url`.
 * @throws {Error} naming the URL and the status, when the server does not answer 2xx.
 */
export async function fetchOk(url: string | URL): Promise<Response> {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`GET ${String(url)}: ${String(response.status)}`);
  }
  return response;
}
