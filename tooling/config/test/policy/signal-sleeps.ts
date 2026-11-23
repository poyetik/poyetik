/** Sleeps by a signal that times out, and by blocking the thread; not a signal a test aborts itself, nor a notification. */
export function sleeps(cell: Int32Array<SharedArrayBuffer>): readonly unknown[] {
  return [AbortSignal.timeout(10), Atomics.wait(cell, 0, 0, 10), AbortSignal.abort(), Atomics.notify(cell, 0)];
}
