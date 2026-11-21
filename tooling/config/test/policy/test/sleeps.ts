/** A sleep in a unit test's folder, and `fetch`, refused in test code as elsewhere. */
export async function sleeps(): Promise<Response> {
  await new Promise((resolve) => {
    setTimeout(resolve, 10);
  });
  return fetch("/x");
}
