/** A sleep in an end-to-end test file. */
export async function sleeps(): Promise<void> {
  await new Promise((resolve) => {
    setTimeout(resolve, 10);
  });
}
