// Keyboard and focus helpers for the end-to-end tests: targets are reached the way a keyboard user
// reaches them, and the focus ring is measured as the browser draws it.
import { expect, type Locator, type Page } from "@playwright/test";

/** More Tab presses than the page has focusable elements: a target not reached by then never will be. */
const MAX_TABS = 100;

/** Focuses `target` by pressing Tab from the start of the page, as a keyboard user would, and fails if Tab never reaches it. */
export async function tabTo(page: Page, target: Locator): Promise<void> {
  await page.locator("body").focus();
  for (let presses = 0; presses < MAX_TABS; presses += 1) {
    await page.keyboard.press("Tab");
    if (await target.evaluate((element) => element === document.activeElement)) {
      await expect(target).toBeFocused();
      return;
    }
  }
  throw new Error(`Tab never reached the target within ${String(MAX_TABS)} presses`);
}

/** The relative luminance of an sRGB colour given as 0-255 channels (WCAG 2.2, definition of relative luminance). */
export function luminance([red, green, blue]: readonly [number, number, number]): number {
  const linear = (channel: number): number => {
    const value = channel / 255;
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * linear(red) + 0.7152 * linear(green) + 0.0722 * linear(blue);
}

/** The contrast ratio of two colours (WCAG 2.2, definition of contrast ratio): from 1 to 21. */
export function contrast(first: readonly [number, number, number], second: readonly [number, number, number]): number {
  const [light, dark] = [luminance(first), luminance(second)].sort((left, right) => right - left);
  return ((light ?? 0) + 0.05) / ((dark ?? 0) + 0.05);
}

/** The channels of a computed `rgb(r, g, b)` or opaque `rgba(r, g, b, 1)` colour; fails on anything else. */
export function channels(color: string): [number, number, number] {
  const match = /^rgba?\((\d+), (\d+), (\d+)(?:, 1)?\)$/.exec(color);
  if (match === null) {
    throw new Error(`not an opaque rgb() colour: ${color}`);
  }
  return [Number(match[1]), Number(match[2]), Number(match[3])];
}
