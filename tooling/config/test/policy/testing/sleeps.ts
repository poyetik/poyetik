/** A sleep in a package's testing folder, test code too. */
export async function sleeps(): Promise<void> {
  await new Promise((resolve) => {
    setTimeout(resolve, 10);
  });
}
