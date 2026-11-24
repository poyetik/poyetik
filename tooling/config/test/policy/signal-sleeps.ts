/** Sleeps by a signal that times out, and by blocking the thread; not a signal a test aborts itself, nor a notification. */
export function sleeps(cell: Int32Array<SharedArrayBuffer>): readonly unknown[] {
  return [AbortSignal.timeout(10), Atomics.wait(cell, 0, 0, 10), AbortSignal.abort(), Atomics.notify(cell, 0), Atomics.waitAsync(cell, 0, 0, 10)];
}

/** The same sleeps through each global object (sixth deep review of visual block 2). */
export function globalSleeps(cell: Int32Array<SharedArrayBuffer>): readonly unknown[] {
  return [
    globalThis.AbortSignal.timeout(10),
    globalThis.Atomics.wait(cell, 0, 0, 10),
    globalThis.Atomics.waitAsync(cell, 0, 0, 10),
    window.AbortSignal.timeout(10),
    window.Atomics.wait(cell, 0, 0, 10),
    window.Atomics.waitAsync(cell, 0, 0, 10),
    self.AbortSignal.timeout(10),
    self.Atomics.wait(cell, 0, 0, 10),
    self.Atomics.waitAsync(cell, 0, 0, 10),
    globalThis.AbortSignal.abort(),
    globalThis.Atomics.notify(cell, 0),
  ];
}
