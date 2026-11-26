// The page built from the fixture board (e2e/fixture-board.json, astro.fixture.config.mjs):
// every rendering branch of a column is reached and audited here, whatever board.json holds,
// so that a data-only change of the board never decides which branch a test runs.
import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import type { ColumnId } from "../src/lib/board.ts";
import { channels, contrast } from "./contrast.ts";
import { tabTo } from "./keyboard.ts";

/** The port of the fixture page's preview, chosen by playwright.config.ts. */
const PORT = process.env["ROADMAP_E2E_FIXTURE_PORT"];
if (PORT === undefined) {
  throw new Error("ROADMAP_E2E_FIXTURE_PORT is not set: run through playwright.config.ts");
}

test.use({ baseURL: `http://localhost:${PORT}/` });

/** The states a column renders in today, listed by hand from Board.astro, each with the fixture column that holds it. */
const STATES: Readonly<Record<"empty" | "all-shown" | "with-more", ColumnId>> = { empty: "later", "all-shown": "doing", "with-more": "done" };

/** Opens the fixture page and checks that it is the fixture's, before anything depends on it. */
async function openFixture(page: Page): Promise<void> {
  await page.goto("./");
  // Written as literals, the facts of e2e/fixture-board.json.
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Fixture");
  await expect(page.locator('[data-tally="done"]')).toHaveText("· 7 Done");
}

test("serves the fixture board, every column state among its columns", async ({ page }) => {
  await openFixture(page);
  await expect(page.locator(`[data-column="${STATES.empty}"] .empty`)).toHaveText("Nothing yet");
  await expect(page.locator(`[data-column="${STATES["all-shown"]}"] details.more`)).toHaveCount(0);
  await expect(page.locator(`[data-column="${STATES["with-more"]}"] details.more`)).toHaveCount(1);
});

test("shows five Done cards, the others behind Show all, named as the user sees it", async ({ page }) => {
  await openFixture(page);
  const section = page.locator(`[data-column="${STATES["with-more"]}"]`);
  const visible = section.locator(".card:visible");
  const toggle = section.locator("details.more > summary");
  const [closed, open] = [toggle.locator(".when-closed"), toggle.locator(".when-open")];
  await expect(visible).toHaveCount(5);
  await expect(toggle).toHaveAccessibleName("Show all 7");
  await expect(closed).toBeVisible();
  await expect(open).toBeHidden();
  expect(await open.evaluate((element) => getComputedStyle(element).display)).toBe("none");
  // Axe in the closed state.
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);
  await toggle.click();
  await expect(toggle).toHaveAccessibleName("Show fewer");
  await expect(open).toBeVisible();
  await expect(closed).toBeHidden();
  expect(await closed.evaluate((element) => getComputedStyle(element).display)).toBe("none");
  await expect(visible).toHaveCount(7);
  // Most recent first: the fixture's cards were finished on days 1 to 7.
  expect(await visible.evaluateAll((cards) => cards.map((card) => card.getAttribute("data-id")))).toStrictEqual(["9", "8", "7", "6", "5", "4", "3"]);
  // Axe in the open state.
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);
});

test("draws a visible focus ring on Show all", async ({ page }) => {
  await openFixture(page);
  const summary = page.locator("details.more > summary");
  await tabTo(page, summary);
  const ring = await summary.evaluate((element) => {
    const style = getComputedStyle(element);
    return { style: style.outlineStyle, width: style.outlineWidth, color: style.outlineColor, background: getComputedStyle(document.body).backgroundColor };
  });
  expect(ring.style).toBe("solid");
  expect(Number.parseFloat(ring.width)).toBeGreaterThanOrEqual(2);
  expect(contrast(channels(ring.color), channels(ring.background))).toBeGreaterThanOrEqual(3);
});

test("shows the progress of an In progress card on its closed line", async ({ page }) => {
  await openFixture(page);
  const card = page.locator(`[data-column="doing"] [data-id="2"]`);
  const summary = card.locator("details.card-toggle > summary");
  await expect(card.locator("details.card-toggle")).not.toHaveAttribute("open", "");
  await expect(summary.locator(".progress")).toBeVisible();
  await expect(summary).toHaveAccessibleName("Work under way Steps 2/4");
  // Beside the title, on its one line.
  const [title, progress] = await Promise.all([summary.locator("h3").boundingBox(), summary.locator(".progress").boundingBox()]);
  expect(Math.abs((title?.y ?? 0) + (title?.height ?? 0) - ((progress?.y ?? 0) + (progress?.height ?? 0)))).toBeLessThan(4);
  // Open, the card shows its progress once, in its steps' toggle.
  await summary.click();
  await expect(card.locator("details.card-toggle")).toHaveAttribute("open", "");
  await expect(summary.locator(".progress")).toBeHidden();
  await expect(card.locator("details.steps-toggle > summary")).toContainText("Steps 2/4");
});

test("names a card by its title alone, its marker apart, closed and open, and its steps as a list", async ({ page }) => {
  await openFixture(page);
  const card = page.locator(`[data-id="1"]`);
  const summary = card.locator("details.card-toggle > summary");
  await expect(summary).toHaveAccessibleName("A card without description or milestone");
  await summary.click();
  await expect(summary).toHaveAccessibleName("A card without description or milestone");
  // Without description or milestone: neither is rendered.
  await expect(card.locator("p")).toHaveCount(0);
  await expect(card.locator(".milestone")).toHaveCount(0);
  const doing = page.locator(`[data-id="2"]`);
  await doing.locator("details.card-toggle > summary").click();
  await doing.locator("details.steps-toggle > summary").click();
  await expect(doing.getByRole("list", { name: "Steps" })).toBeVisible();
});

test("groups a card's steps by review block, each list named by its heading, and says once what ends a block", async ({ page }) => {
  await openFixture(page);
  const card = page.locator(`[data-column="doing"] [data-id="10"]`);
  await card.locator("details.card-toggle > summary").click();
  await card.locator("details.steps-toggle > summary").click();
  // Written as literals, the facts of the fixture's card #10.
  await expect(card.locator(".block h4")).toHaveText(["Block 1: steps 1 and 3 Done", "Block 2: step 2", "Not in a block"]);
  for (const [name, steps] of [
    ["Block 1: steps 1 and 3 Done", ["Block one, first step Done", "Block one, a later step Done"]],
    ["Block 2: step 2", ["Block two, only step In progress"]],
    ["Not in a block", ["A step in no block yet"]],
  ] as const) {
    await expect(card.getByRole("list", { name, exact: true }).getByRole("listitem")).toHaveText(steps);
  }
  expect(await card.locator(".steps .step").evaluateAll((items) => items.map((item) => item.getAttribute("value")))).toStrictEqual(["1", "3", "2", "4"]);
  // The numbers are shown: each item a list item with a decimal marker, as the page's stylesheet says.
  expect(
    await card.locator(".steps .step").evaluateAll((items) =>
      items.map((item) => {
        const style = getComputedStyle(item);
        return `${style.display} ${style.listStyleType}`;
      }),
    ),
  ).toStrictEqual(["list-item decimal", "list-item decimal", "list-item decimal", "list-item decimal"]);
  await expect(card.locator(".block-end")).toHaveText(["Each block ends with a deep review, its decided fixes, and a pull request whose full check is green."]);
  // No landmark per block: the card's column is a region, the card holds none. Read on the column, not
  // the whole page: in Playwright's Firefox (1.65), its aria engine over the whole page, then axe, crash
  // the page's process (a snapshot of the body, then axe, crashes it alone), where neither crashes it
  // alone and the page runs no script of its own.
  await expect(page.locator('[data-column="doing"]')).toHaveRole("region");
  await expect(card.getByRole("region")).toHaveCount(0);
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);
});
