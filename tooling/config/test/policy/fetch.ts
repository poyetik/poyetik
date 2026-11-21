/** A package's checked helper, the one file allowed to call `fetch`. */
export async function fetchOk(url: string): Promise<Response> {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`GET ${url}: ${String(response.status)}`);
  }
  return response;
}

/** A fetch helper that is no test code may run a timer. */
export function later(check: () => void): number {
  return setTimeout(check, 10);
}
