/** What waits for no time: a signal a test aborts itself, and a notification, bare and through `globalThis` (seventh deep review of visual block 2). */
export function waits(cell: Int32Array<SharedArrayBuffer>): readonly unknown[] {
  return [AbortSignal.abort(), Atomics.notify(cell, 0), globalThis.AbortSignal.abort(), globalThis.Atomics.notify(cell, 0)];
}
