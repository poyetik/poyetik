import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";
import { readFileSync } from "node:fs";
import { parseBoard } from "../src/lib/board.ts";
import { toView } from "../src/lib/view.ts";

const view = toView(parseBoard(JSON.parse(readFileSync(new URL("../board.json", import.meta.url), "utf8"))));

test("shows every board.json card in its column", async ({ page }) => {
  await page.goto("./");
  await expect(page).toHaveTitle(`${view.title} · Roadmap`);
  await expect(page.getByRole("heading", { level: 1 })).toHaveText(view.title);
  for (const column of view.columns) {
    const section = page.locator(`[data-column="${column.id}"]`);
    await expect(section.getByRole("heading", { level: 2 })).toContainText(column.title);
    await expect(section.locator(".card")).toHaveCount(column.cards.length);
    for (const card of column.cards) {
      await expect(section.locator(`[data-id="${String(card.id)}"] h3`)).toHaveText(card.title);
    }
  }
});

test("ships no JavaScript", async ({ page }) => {
  const scripts: string[] = [];
  page.on("request", (request) => {
    if (request.resourceType() === "script") {
      scripts.push(request.url());
    }
  });
  await page.goto("./");
  expect(scripts).toEqual([]);
  await expect(page.locator("script")).toHaveCount(0);
});

test("passes accessibility rules (axe)", async ({ page }) => {
  await page.goto("./");
  const results = await new AxeBuilder({ page }).analyze();
  expect(results.violations).toEqual([]);
});

test("stays readable in dark mode", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("./");
  const results = await new AxeBuilder({ page }).withTags(["wcag2aa"]).analyze();
  expect(results.violations).toEqual([]);
});
