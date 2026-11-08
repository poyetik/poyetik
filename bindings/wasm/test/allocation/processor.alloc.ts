// The real audio-thread entry point, from a fresh processor, at every quantum of the
// matrix that is a whole number of blocks: run in every V8 tier (the "every tier" project)
// and in the interpreter alone (the "interpreter" project), where nothing is optimized away.
import { expect, test } from "vitest";
import { HOST_BUFFER_SIZES } from "../fixtures.ts";
import { BLOCK, processing } from "./processing.ts";

test.each(HOST_BUFFER_SIZES.filter((size) => size % BLOCK === 0))("processing quanta of %i frames does not allocate", async (size) => {
  // 100 000 blocks.
  expect(await processing(size, 6_400_000)).toBe(0);
});
