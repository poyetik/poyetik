/** Reads a response whose status nobody checked: a 404's empty body would be parsed as JSON. */
export async function load(): Promise<unknown> {
  const response = await fetch("/case.json");
  return response.json();
}

/** The same global through its objects. */
export async function loadAgain(): Promise<Response[]> {
  return Promise.all([globalThis.fetch("/a"), window.fetch("/b"), self.fetch("/c")]);
}
