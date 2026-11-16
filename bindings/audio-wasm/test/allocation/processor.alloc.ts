// The real audio-thread entry point, from the first quantum of a fresh processor, at every
// quantum of the matrix that is a whole number of blocks: run in every V8 tier (the "every
// tier" project) and in the interpreter alone (the "interpreter" project), where nothing is
// optimized away; and the tap processor, which allocates. Both run with a young generation of 1 MB, so that little garbage is
// enough to cause a collection; the control shows the count sees one object per quantum.
import { expect, test } from "vitest";
import { HOST_BUFFER_SIZES } from "../fixtures.ts";
import { BLOCK, OUTPUTS, processing, tapping } from "./processing.ts";

/** The last object made by the control: stored, so that no tier can optimize it away. */
let kept: object = {};

test("the count sees garbage: one object per quantum of 128 frames collects", async () => {
  const before = kept;
  // 50 000 quanta: the same run as the test below at 128 frames.
  expect(
    await processing(128, 6_400_000, () => {
      kept = {};
    }),
  ).toBeGreaterThan(0);
  expect(kept).not.toBe(before);
});

// Through every mode of the output stage: the limiter runs in the module, its quanta copied
// as a raw stage's are. A limited stage's run is a fixed amount of work, which took about
// 2.1 s in both projects on a 10-core Mac at a load average of 15, and up to 13.9 s under
// heavier load (2026-11-16), over Vitest's 5 s default; 60 s leaves a margin of over 4
// times the slowest run seen.
test.each(Object.entries(OUTPUTS).flatMap(([mode, output]) => HOST_BUFFER_SIZES.filter((size) => size % BLOCK === 0).map((size) => [size, mode, output] as const)))(
  "processing quanta of %i frames through a %s output stage does not allocate",
  async (size, _mode, output) => {
    // 100 000 blocks.
    expect(await processing(size, 6_400_000, undefined, output)).toBe(0);
  },
  60_000,
);

// A known deviation, recorded in CLAUDE.md: the tap processor copies every render quantum
// it posts, which the playground's oscilloscope runs while it plays, until #12 step 21
// replaces it. This test fails, and the record must change, the day it stops.
test("the tap processor allocates: 22 500 quanta of 128 frames collect", async () => {
  expect(await tapping(22_500)).toBeGreaterThan(0);
});
