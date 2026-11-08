// Color contrast as WCAG 2.2 defines it, for the end-to-end tests that measure the focus
// ring as the browser draws it: pure functions, unit-tested (test/contrast.test.ts).

/** The relative luminance of an sRGB color given as 0-255 channels (WCAG 2.2, definition of relative luminance). */
export function luminance([red, green, blue]: readonly [number, number, number]): number {
  const linear = (channel: number): number => {
    const value = channel / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue);
}

/** The contrast ratio of two colors (WCAG 2.2, definition of contrast ratio): from 1 to 21. */
export function contrast(first: readonly [number, number, number], second: readonly [number, number, number]): number {
  const [one, other] = [luminance(first), luminance(second)];
  return (Math.max(one, other) + 0.05) / (Math.min(one, other) + 0.05);
}

/** The channels of a computed `rgb(r, g, b)` or opaque `rgba(r, g, b, 1)` color; fails on anything else. */
export function channels(color: string): [number, number, number] {
  const match = /^rgba?\((\d+), (\d+), (\d+)(?:, 1)?\)$/.exec(color);
  if (match === null) {
    throw new Error(`not an opaque rgb() color: ${color}`);
  }
  return [Number(match[1]), Number(match[2]), Number(match[3])];
}
