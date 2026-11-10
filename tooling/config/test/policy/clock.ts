/** Node's process, which the fixtures' types leave out. */
declare const process: { hrtime: () => readonly [number, number] };

/** Reads every clock the policy refuses outside its clock readers. */
export function now(): readonly unknown[] {
  return [performance.now(), Date.now(), process.hrtime(), new Date(), Date()];
}

/** A date from a timestamp reads no clock. */
export const EPOCH = new Date(0);
