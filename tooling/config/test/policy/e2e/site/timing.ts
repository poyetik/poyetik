/** A clock reader: the bench page's timing module may read the clock, and nothing else it refuses. */
export function now(): readonly unknown[] {
  return [performance.now(), Date.now(), new Date(), Promise.withResolvers<number>()];
}
