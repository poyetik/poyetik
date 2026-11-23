// A page module whose `page_image_at` answers outside its contract: the answer is refused
// with a `PageModuleError`, outside the driver's guard, so that the instance, which did not
// fail, stays usable. The real module never answers so (an i64 export answers a BigInt):
// its page interface is replaced, for this file only, by one answering a number.
import { TEST_SQUARE } from "@poyetik/visual-wasm/testing/conformance";
import { expect, test, vi } from "vitest";
import { type Guard, type ImageAt, PageModuleError } from "../ts/interface.ts";
import { PageModule } from "../ts/page.ts";
import { pageBytes } from "./modules.ts";

vi.mock(import("../ts/interface.ts"), async (original) => {
  const real = await original();
  return {
    ...real,
    /**
     * The real page interface, answering a number instead of an image.
     * @param exports The exports of an instance of the page module.
     * @param guard The driver's guard.
     * @returns `page_image_at`, answering 2.
     */
    pageInterface: (exports: Readonly<Record<string, unknown>>, guard: Guard): ImageAt => {
      const imageAt = real.pageInterface(exports, guard);
      return (frame, sampleRate, numerator, denominator) => Number(imageAt(frame, sampleRate, numerator, denominator));
    },
  };
});

test("refuses an answer that is not an image, the instance still usable", async () => {
  const page = await PageModule.instantiate(pageBytes(), TEST_SQUARE);
  expect(() => page.imageAt(1600n, 48_000, [60, 1])).toThrow(new PageModuleError("page_image_at did not answer a 64-bit integer"));
  expect(page.render(7n), "the instance did not fail: still called").toHaveLength(17 * 9 * 4);
});
