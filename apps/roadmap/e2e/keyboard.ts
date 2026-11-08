// Keyboard helpers for the end-to-end tests: targets are reached the way a keyboard user
// reaches them. The focus ring is measured with contrast.ts.
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
