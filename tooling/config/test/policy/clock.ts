/** Node's process, which the fixtures' types leave out. */
declare const process: { hrtime: () => readonly [number, number]; uptime: () => number };

/** Reads every clock the policy refuses outside its clock readers. */
export function now(): readonly unknown[] {
  return [performance.now(), Date.now(), process.hrtime(), process.uptime(), new Date(), Date()];
}

/** The same clocks through the global object: each form by each of its names. */
export function nowThroughTheGlobal(): readonly unknown[] {
  return [
    globalThis.performance.now(),
    window.performance.now(),
    self.performance.now(),
    globalThis.Date.now(),
    window.Date.now(),
    self.Date.now(),
    new globalThis.Date(),
    new window.Date(),
    new self.Date(),
    globalThis.Date(),
    window.Date(),
    self.Date(),
  ];
}

/** A date from a timestamp reads no clock. */
export const EPOCH = new Date(0);

/** Nor through the global object. */
export const GLOBAL_EPOCH = new globalThis.Date(0);
