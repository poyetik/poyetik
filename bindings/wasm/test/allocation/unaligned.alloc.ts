// The real audio-thread entry point, from a fresh processor, at every quantum of the
// matrix that is not a whole number of blocks: run in the interpreter alone (the
// "interpreter" project), where the result does not depend on when V8 optimizes.
import { expect, test } from "vitest";
import { HOST_BUFFER_SIZES } from "../fixtures.ts";
import { BLOCK, processing } from "./processing.ts";

// A known deviation, recorded in CLAUDE.md: a quantum that is not a whole number of blocks
// is copied sample by sample, which boxes numbers until the code is optimized. Browsers
// render quanta of 128 frames today; this test fails, and the record must change, the day
// such quanta stop allocating.
test.each(HOST_BUFFER_SIZES.filter((size) => size % BLOCK !== 0))("processing quanta of %i frames, not whole blocks, allocates in the interpreter", async (size) => {
  // 10 000 blocks: enough to see the garbage, sample by sample in the interpreter.
  expect(await processing(size, 640_000)).toBeGreaterThan(0);
});
