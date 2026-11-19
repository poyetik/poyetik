// A scheduler of repaints the tests drive by hand, shared by the components that repaint.

import { act } from "@testing-library/react";

/** A scheduler driven by the test: `tick` runs one repaint; counts the schedules and cancels. */
export function manual(): { schedule: (frame: () => void) => () => void; tick: () => void; scheduled: () => number; cancelled: () => number } {
  let current: (() => void) | undefined;
  let scheduled = 0;
  let cancelled = 0;
  return {
    schedule: (frame) => {
      current = frame;
      scheduled += 1;
      return () => {
        current = undefined;
        cancelled += 1;
      };
    },
    tick: () => {
      act(() => {
        current?.();
      });
    },
    scheduled: () => scheduled,
    cancelled: () => cancelled,
  };
}
