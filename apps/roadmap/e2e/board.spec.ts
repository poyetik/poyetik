import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";
import { readFileSync } from "node:fs";
import { parseBoard } from "../src/lib/board.ts";
import { toView } from "../src/lib/view.ts";
import { CSP } from "../src/lib/csp.ts";

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

test("is dark and stays readable whatever the system theme", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("./");
  await expect(page.locator("body")).toHaveCSS("background-color", "rgb(22, 22, 21)");
  const results = await new AxeBuilder({ page }).withTags(["wcag2aa"]).analyze();
  expect(results.violations).toEqual([]);
});

test("steps are collapsed by default and expand on click", async ({ page }) => {
  const withSteps = view.columns.flatMap((c) => c.cards).find((c) => c.steps.length > 0);
  if (withSteps === undefined) {
    throw new Error("board.json must contain at least one card with steps for this test");
  }
  await page.goto("./");
  const card = page.locator(`[data-id="${String(withSteps.id)}"]`);
  const steps = card.locator(".step");
  await expect(card.locator("summary")).toContainText(`Steps ${String(withSteps.doneSteps)}/${String(withSteps.steps.length)}`);
  await expect(steps.first()).toBeHidden();
  await card.locator("summary").click();
  await expect(steps).toHaveCount(withSteps.steps.length);
  await expect(steps.first()).toBeVisible();
});

test("declares its Content Security Policy, breaks none of it, and enforces it", async ({ page }) => {
  await page.addInitScript(() => {
    const violations: string[] = [];
    Reflect.set(window, "violations", violations);
    document.addEventListener("securitypolicyviolation", (event) => {
      violations.push(`${event.violatedDirective} ${event.blockedURI}`);
    });
  });
  await page.goto("./");
  await expect(page.locator('meta[http-equiv="Content-Security-Policy"]')).toHaveAttribute("content", CSP);
  await expect(page.locator("style")).toHaveCount(0);
  const violations = (): Promise<unknown> => page.evaluate((): unknown => Reflect.get(window, "violations"));
  expect(await violations()).toStrictEqual([]);
  // Control: an image from another site is blocked before any request.
  await page.evaluate(() => {
    const image = document.createElement("img");
    image.src = "https://example.invalid/blocked.png";
    document.body.append(image);
  });
  await expect.poll(violations).toStrictEqual(["img-src https://example.invalid/blocked.png"]);
});

test("loads without a failed request or a console error (its favicon included)", async ({ page }) => {
  const errors: string[] = [];
  page.on("console", (message) => {
    if (message.type() === "error") {
      errors.push(message.text());
    }
  });
  page.on("response", (response) => {
    if (response.status() >= 400) {
      errors.push(`HTTP ${String(response.status())}: ${response.url()}`);
    }
  });
  await page.goto("./");
  await expect(page.locator('link[rel="icon"]')).toHaveAttribute("href", "/favicon.svg");
  expect((await page.request.get("/favicon.svg")).status()).toBe(200);
  expect(errors).toStrictEqual([]);
});
