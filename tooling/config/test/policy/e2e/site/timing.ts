import { describe } from "vitest";

/** A clock reader: the bench page's timing module may read the clock, and nothing else it refuses. */
export function now(): readonly unknown[] {
  return [performance.now(), Date.now(), new Date(), globalThis.performance.now(), new globalThis.Date(), Promise.withResolvers<number>()];
}

/** Every restricted syntax but a clock read stays refused in a clock reader: an enum… */
export enum Side {
  Left,
}

/** …a global augmentation… */
declare global {
  /** Doc. */
  interface Window {
    /** Doc. */
    extra: number;
  }
}

// …and a skipped test.
describe.skip("a skipped suite", () => undefined);
