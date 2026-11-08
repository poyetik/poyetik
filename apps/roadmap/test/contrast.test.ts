import { describe, expect, test } from "vitest";
import { channels, contrast, luminance } from "../e2e/contrast.ts";

/** Pure white and pure black, the extremes of contrast. */
const WHITE = [255, 255, 255] as const;
/** See {@link WHITE}. */
const BLACK = [0, 0, 0] as const;

describe("luminance", () => {
  test("is 1 for white and 0 for black", () => {
    expect(luminance(WHITE)).toBe(1);
    expect(luminance(BLACK)).toBe(0);
  });

  test("weighs red, green and blue as WCAG 2.2 does", () => {
    expect(luminance([255, 0, 0])).toBeCloseTo(0.2126, 12);
    expect(luminance([0, 255, 0])).toBeCloseTo(0.7152, 12);
    expect(luminance([0, 0, 255])).toBeCloseTo(0.0722, 12);
  });

  test("is linear up to 0.04045 of a channel, a power curve above", () => {
    // 10/255 = 0.0392 is below the threshold; 11/255 = 0.0431 above it.
    expect(luminance([10, 10, 10])).toBeCloseTo(10 / 255 / 12.92, 12);
    expect(luminance([11, 11, 11])).toBeCloseTo(((11 / 255 + 0.055) / 1.055) ** 2.4, 12);
    // The threshold itself is on the linear side (0.04045 × 255 / 255 is exactly 0.04045);
    // the power curve gives 2.3e-9 more there, far beyond 5e-13, the precision asked.
    const threshold = 0.04045 * 255;
    expect(luminance([threshold, threshold, threshold])).toBeCloseTo(0.04045 / 12.92, 12);
  });
});

describe("contrast", () => {
  test("is 21 between white and black, whichever comes first", () => {
    expect(contrast(WHITE, BLACK)).toBeCloseTo(21, 12);
    expect(contrast(BLACK, WHITE)).toBeCloseTo(21, 12);
  });

  test("is 1 between a color and itself", () => {
    expect(contrast([118, 118, 118], [118, 118, 118])).toBe(1);
  });

  test("is (L1 + 0.05) / (L2 + 0.05), the lighter over the darker", () => {
    const light = luminance([255, 255, 0]);
    const dark = luminance([0, 0, 128]);
    expect(contrast([0, 0, 128], [255, 255, 0])).toBeCloseTo((light + 0.05) / (dark + 0.05), 12);
  });
});

describe("channels", () => {
  test.each([
    ["rgb(1, 2, 3)", [1, 2, 3]],
    ["rgba(10, 200, 255, 1)", [10, 200, 255]],
    ["rgba(0, 0, 0)", [0, 0, 0]],
  ] as const)("reads %s", (color, expected) => {
    expect(channels(color)).toStrictEqual(expected);
  });

  test.each(["rgba(1, 2, 3, 0.5)", "rgb(1,2,3)", "xrgb(1, 2, 3)", "rgb(1, 2, 3)x", "rgb(1, 2)", "#010203", "rgb(a, 2, 3)"])("refuses %s", (color) => {
    expect(() => channels(color)).toThrow(new Error(`not an opaque rgb() color: ${color}`));
  });
});
