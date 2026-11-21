/** A sleep in a unit test file. */
export async function sleeps(): Promise<void> {
  await new Promise((resolve) => {
    setTimeout(resolve, 10);
  });
}
