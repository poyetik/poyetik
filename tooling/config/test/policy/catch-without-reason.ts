/** Doc. */
export async function closed(closing: Promise<void>): Promise<void> {
  await closing.catch(() => undefined);
}
