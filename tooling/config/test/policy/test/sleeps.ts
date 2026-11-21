/** A sleep in a unit test's folder. */
export async function sleeps(): Promise<void> {
  await new Promise((resolve) => {
    setTimeout(resolve, 10);
  });
}
