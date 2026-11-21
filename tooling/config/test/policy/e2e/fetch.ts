/** A checked fetch helper in test code: it may call `fetch`, never sleep. */
export async function fetchOk(url: string): Promise<Response> {
  const response = await fetch(url);
  await new Promise((resolve) => {
    setTimeout(resolve, 1);
  });
  return response;
}
