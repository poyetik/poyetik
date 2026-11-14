// The real audio-thread entry point, from a fresh processor, at every quantum of the
// matrix that is not a whole number of blocks: run in the interpreter alone (the
// "interpreter" project), where the result does not depend on when V8 optimizes.
import { expect, test } from "vitest";
import { HOST_BUFFER_SIZES } from "../fixtures.ts";
import { BLOCK, processing } from "./processing.ts";

// A known deviation, recorded in CLAUDE.md: a quantum that is not a whole number of blocks
// copies part of a block through a view of that part (`subarray`), one allocation per
// partial copy, at least until V8 optimizes it (the optimized tiers are not measured).
// Browsers render quanta of 128 frames today; this test fails, and the record must
// change, the day such quanta stop allocating.
test.each(HOST_BUFFER_SIZES.filter((size) => size % BLOCK !== 0))("processing quanta of %i frames, not whole blocks, allocates in the interpreter", async (size) => {
  // 10 000 blocks: enough to see the garbage of the views in the interpreter.
  expect(await processing(size, 640_000)).toBeGreaterThan(0);
});
