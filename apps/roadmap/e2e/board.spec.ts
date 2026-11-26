import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { parseBoard } from "../src/lib/board.ts";
import { toView } from "../src/lib/view.ts";
import { CSP } from "../src/lib/csp.ts";
import { LOCALES, TIME_ZONES } from "../test/matrix.ts";
import { channels, contrast } from "./contrast.ts";
import { tabTo } from "./keyboard.ts";

/** The expected display, computed from the real board.json with the same code as the page, so the tests follow the data instead of hard-coding cards. */
const view = toView(parseBoard(JSON.parse(readFileSync(new URL("../board.json", import.meta.url), "utf8"))));

/** Opens card `id` with clicks, as a user would: its column's "Show all" first when the card is behind it, then the card itself. */
async function openCard(page: Page, id: number): Promise<void> {
  const card = page.locator(`[data-id="${String(id)}"]`);
  const more = page.locator("details.more", { has: card });
  if ((await more.count()) > 0 && (await more.getAttribute("open")) === null) {
    await more.locator("> summary").click();
  }
  await card.locator("details.card-toggle > summary").click();
  await expect(card.locator("details.card-toggle")).toHaveAttribute("open", "");
}

/**
 * Opens every toggle of the page with clicks, outermost first: every "Show all", every card,
 * then every card's steps. Each click must open its toggle: one that does not fails at once,
 * never a loop until the test's timeout.
 */
async function openEverything(page: Page): Promise<void> {
  for (const selector of ["details.more", "details.card-toggle", "details.steps-toggle"]) {
    const toggles = page.locator(`${selector}:not([open]) > summary`);
    for (let closed = await toggles.count(); closed > 0; closed -= 1) {
      await toggles.first().click();
      await expect(toggles).toHaveCount(closed - 1);
    }
  }
  await expect(page.locator("details:not([open])")).toHaveCount(0);
}

test("shows every board.json card in its column", async ({ page }) => {
  await page.goto("./");
  // The project's name is a fixed fact, not data: literal, so that a wrong board title fails.
  await expect(page).toHaveTitle("Poyetik · Roadmap");
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Poyetik");
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
  const withSteps = view.columns.flatMap((column) => column.cards).find((candidate) => candidate.steps.length > 0);
  if (withSteps === undefined) {
    throw new Error("board.json must contain at least one card with steps for this test");
  }
  await page.goto("./");
  await openCard(page, withSteps.id);
  const card = page.locator(`[data-id="${String(withSteps.id)}"]`);
  const steps = card.locator(".step");
  const summary = card.locator("details.steps-toggle > summary");
  await expect(summary).toContainText(`Steps ${String(withSteps.doneSteps)}/${String(withSteps.steps.length)}`);
  await expect(steps.first()).toBeHidden();
  await summary.click();
  await expect(steps).toHaveCount(withSteps.steps.length);
  await expect(steps.first()).toBeVisible();
});

test("declares its Content Security Policy, breaks none of it, and enforces it", async ({ page }) => {
  await page.addInitScript(() => {
    const seen: string[] = [];
    Reflect.set(window, "violations", seen);
    document.addEventListener("securitypolicyviolation", (event) => {
      seen.push(`${event.violatedDirective} ${event.blockedURI}`);
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

test("passes accessibility rules with every card and its steps expanded", async ({ page }) => {
  await page.goto("./");
  expect(await page.locator("details.steps-toggle").count()).toBeGreaterThan(0);
  await openEverything(page);
  expect((await new AxeBuilder({ page }).analyze()).violations).toStrictEqual([]);
});

test("collapses every card to its title on one line, cut with an ellipsis, and opens it on click", async ({ page }) => {
  await page.goto("./");
  const all = view.columns.flatMap((column) => column.cards);
  await expect(page.locator("details.card-toggle")).toHaveCount(all.length);
  await expect(page.locator("details.card-toggle[open]")).toHaveCount(0);
  // A title far longer than a card is wide, on the first card.
  const first = all[0];
  if (first === undefined) {
    throw new Error("board.json must contain at least one card for this test");
  }
  const card = page.locator(`[data-id="${String(first.id)}"]`);
  const title = card.locator("h3");
  const long = Array.from({ length: 40 }, () => "word").join(" ");
  await title.evaluate((element, text) => {
    element.textContent = text;
  }, long);
  const measure = (): Promise<{ lines: number; cut: boolean; ellipsis: string }> =>
    title.evaluate((element) => {
      const style = getComputedStyle(element);
      return {
        lines: Math.round(element.getBoundingClientRect().height / Number.parseFloat(style.lineHeight === "normal" ? String(Number.parseFloat(style.fontSize) * 1.2) : style.lineHeight)),
        cut: element.scrollWidth > element.clientWidth,
        ellipsis: style.textOverflow,
      };
    });
  expect(await measure()).toStrictEqual({ lines: 1, cut: true, ellipsis: "ellipsis" });
  // Cut, never wider than its column: a title kept on one line must not widen the card.
  const overflowing = await page.locator(".card").evaluateAll((cards) =>
    cards.filter((item) => item.getBoundingClientRect().right > (item.closest(".column")?.getBoundingClientRect().right ?? 0)).map((item) => item.getAttribute("data-id")),
  );
  expect(overflowing).toStrictEqual([]);
  // The whole title stays in the page, for screen readers and search.
  await expect(title).toHaveText(long);
  await expect(card.locator("footer")).toBeHidden();
  await openCard(page, first.id);
  const opened = await measure();
  expect(opened.lines).toBeGreaterThan(1);
  expect(opened.cut).toBe(false);
  await expect(card.locator("footer")).toBeVisible();
  await expect(card.locator(".id")).toHaveText(`#${String(first.id)}`);
});

test("shows the most recent Done cards first, as the view orders them", async ({ page }) => {
  await page.goto("./");
  const done = view.columns.find((column) => column.id === "done");
  if (done === undefined) {
    throw new Error("the board has a Done column");
  }
  // Whatever board.json holds: the cards shown, then those behind Show all (whose toggle
  // e2e/fixture.spec.ts proves on a board that always has one).
  const ids = await page.locator('[data-column="done"] .card').evaluateAll((cards) => cards.map((card) => card.getAttribute("data-id")));
  expect(ids).toStrictEqual([...done.shown, ...done.more].map((card) => String(card.id)));
  await expect(page.locator('[data-column="done"] .card:visible')).toHaveCount(done.shown.length);
});

test("sums up the board with the number of cards of each column", async ({ page }) => {
  await page.goto("./");
  const expected = view.columns.map((column) => `${String(column.cards.length)} ${column.title}`).join(" · ");
  await expect(page.getByRole("region", { name: "Progress", exact: true })).toContainText(expected);
  await expect(page.getByRole("region", { name: "Progress", exact: true })).not.toContainText(" / ");
});

test("shows the columns in classic kanban order from left to right", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("./");
  const lefts = [];
  for (const id of ["later", "next", "doing", "done"]) {
    const box = await page.locator(`[data-column="${id}"]`).boundingBox();
    lefts.push(box?.x ?? Number.NaN);
  }
  expect(lefts).toStrictEqual([...lefts].sort((left, right) => left - right));
  expect(new Set(lefts).size).toBe(4);
});

test("shows In progress first on a phone, where the columns stack", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("./");
  const tops = [];
  for (const id of ["doing", "later", "next", "done"]) {
    const box = await page.locator(`[data-column="${id}"]`).boundingBox();
    tops.push(box?.y ?? Number.NaN);
  }
  expect(tops).toStrictEqual([...tops].sort((top, bottom) => top - bottom));
  expect(new Set(tops).size).toBe(4);
});

test("runs in a time zone and a locale of the test matrix, not the machine's", async ({ page }, testInfo) => {
  const { timezoneId, locale } = testInfo.project.use;
  expect(TIME_ZONES).toContain(timezoneId);
  expect(LOCALES).toContain(locale);
  await page.goto("./");
  // The emulation took effect in the page itself.
  expect(await page.evaluate(() => Intl.DateTimeFormat().resolvedOptions().timeZone)).toBe(timezoneId);
  // Compared in their maximal form: WebKit drops a region its language implies ("ja-JP" reads "ja").
  const maximal = (tag: string): string => new Intl.Locale(tag).maximize().toString();
  const [language, formatting] = await page.evaluate(() => [navigator.language, Intl.DateTimeFormat().resolvedOptions().locale]);
  const expected = maximal(locale ?? "");
  expect([maximal(language), maximal(formatting)]).toStrictEqual([expected, expected]);
});

test("names each column region by its heading, count and unit", async ({ page }) => {
  await page.goto("./");
  for (const column of view.columns) {
    const count = column.cards.length;
    await expect(page.locator(`[data-column="${column.id}"]`)).toHaveAccessibleName(
      `${column.title} ${String(count)} ${count === 1 ? "card" : "cards"}`,
    );
  }
  // The unit is for screen readers only: it takes no room on screen.
  const unit = page.locator(".count .visually-hidden").first();
  expect(await unit.evaluate((element) => [element.getBoundingClientRect().width, element.getBoundingClientRect().height])).toStrictEqual([1, 1]);
});

test("wraps an unbreakable word inside its card on the narrowest four-column layout", async ({ page }) => {
  await page.setViewportSize({ width: 901, height: 800 });
  await page.goto("./");
  // A token longer than any card is wide, in every text a card shows: a title, a description, a step,
  // a block's heading and what ends a block. Each is found first, so that none is left out unseen.
  const word = "x".repeat(80);
  await openEverything(page);
  const missing = await page.evaluate((text) => {
    const absent: string[] = [];
    for (const selector of [".card h3", ".card p", ".card .step-title", ".card .block h4", ".card .block-end"]) {
      const element = document.querySelector(selector);
      if (element === null) {
        absent.push(selector);
      } else {
        element.textContent = text;
      }
    }
    return absent;
  }, word);
  expect(missing).toStrictEqual([]);
  // Neither inside a card nor out of its column: a long word could otherwise widen the card's list.
  const overflowing = await page.locator(".card").evaluateAll((cards) =>
    cards
      .filter((card) => {
        const column = card.closest(".column")?.getBoundingClientRect().right ?? 0;
        return card.scrollWidth > card.clientWidth || card.getBoundingClientRect().right > column;
      })
      .map((card) => card.getAttribute("data-id")),
  );
  expect(overflowing).toStrictEqual([]);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth)).toBe(true);
});

test("measures contrast as WCAG defines it", () => {
  expect(contrast([0, 0, 0], [255, 255, 255])).toBe(21);
  expect(contrast([255, 255, 255], [0, 0, 0])).toBe(21);
  expect(contrast([22, 22, 21], [22, 22, 21])).toBe(1);
  expect(channels("rgb(138, 168, 255)")).toStrictEqual([138, 168, 255]);
  expect(() => channels("rgba(0, 0, 0, 0.5)")).toThrow(new Error("not an opaque rgb() color: rgba(0, 0, 0, 0.5)"));
});

test("opens a card, then its steps, with the keyboard alone", async ({ page }) => {
  await page.goto("./");
  const card = page.locator(".card", { has: page.locator("details.steps-toggle") }).first();
  await tabTo(page, card.locator("details.card-toggle > summary"));
  await page.keyboard.press("Enter");
  await expect(card.locator("details.card-toggle")).toHaveAttribute("open", "");
  const summary = card.locator("details.steps-toggle > summary");
  const step = card.locator(".step").first();
  await tabTo(page, summary);
  await expect(step).toBeHidden();
  await page.keyboard.press("Enter");
  await expect(step).toBeVisible();
  await page.keyboard.press("Space");
  await expect(step).toBeHidden();
});

test("draws a visible focus ring on every keyboard-focused toggle", async ({ page }) => {
  await page.goto("./");
  // A card and its steps (in a card opened by keyboard); Show all is e2e/fixture.spec.ts's,
  // on a board that always has one. Each is reached by Tab from the previous one, in page
  // order: a click would move the focus where Tab starts from.
  const withSteps = page.locator(".card", { has: page.locator("details.steps-toggle") }).first();
  const toggles = [withSteps.locator("details.card-toggle > summary"), withSteps.locator("details.steps-toggle > summary")];
  for (const summary of toggles) {
    await tabTo(page, summary);
    if (summary === toggles[0]) {
      await page.keyboard.press("Enter");
    }
    const ring = await summary.evaluate((element) => {
      const style = getComputedStyle(element);
      // The ring is drawn over its card: that is the color it must stand out from.
      const behind = element.closest(".card") ?? document.body;
      return { style: style.outlineStyle, width: style.outlineWidth, color: style.outlineColor, background: getComputedStyle(behind).backgroundColor };
    });
    expect(ring.style).toBe("solid");
    expect(Number.parseFloat(ring.width)).toBeGreaterThanOrEqual(2);
    // WCAG 2.4.13 and 1.4.11: at least 3:1 against what surrounds it.
    expect(contrast(channels(ring.color), channels(ring.background))).toBeGreaterThanOrEqual(3);
  }
});
