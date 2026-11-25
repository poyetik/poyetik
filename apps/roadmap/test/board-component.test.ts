import { experimental_AstroContainer as AstroContainer } from "astro/container";
import axe from "axe-core";
import { describe, expect, it } from "vitest";
import { builtinEnvironments } from "vitest/runtime";
import BoardComponent from "../src/components/Board.astro";
import type { Card } from "../src/lib/board.ts";
import { toView } from "../src/lib/view.ts";

/** Builds a valid card with id `id` in `column` and no steps, dated 2026-11-07; `extra` overrides any field. */
function card(id: number, column: Card["column"], extra: Partial<Card> = {}): Card {
  return {
    id,
    title: `Card ${String(id)}`,
    description: "",
    column,
    milestone: "",
    createdAt: "2026-11-07",
    updatedAt: "2026-11-07",
    steps: [],
    ...extra,
  };
}

/**
 * The accessibility violations axe finds in `html`, placed as the page places it (in `main`, under the
 * `h1`), each as `rule: offending markup`. The test file runs in Node, where the Astro container renders;
 * a jsdom document is installed only for the audit. jsdom has no layout, so color contrast is left to
 * the end-to-end audits of the real page.
 */
async function violations(html: string): Promise<string[]> {
  const environment = await builtinEnvironments.jsdom.setup(globalThis, {});
  try {
    document.documentElement.lang = "en";
    document.title = "P · Roadmap";
    document.body.innerHTML = `<header><h1>P</h1></header><main>${html}</main>`;
    const results = await axe.run(document.documentElement, { rules: { "color-contrast": { enabled: false } } });
    return results.violations.map((violation) => `${violation.id}: ${violation.nodes.map((node) => node.html).join(" | ")}`);
  } finally {
    await environment.teardown(globalThis);
  }
}

/** Renders the Board component for a board holding `cards`, through the same `toView` as the page, checks it has no accessibility violation (see `violations`) and returns its HTML. */
async function render(cards: Card[]): Promise<string> {
  const container = await AstroContainer.create();
  const view = toView({ version: 1, title: "P", nextId: cards.length + 1, cards });
  const html = await container.renderToString(BoardComponent, { props: { view } });
  expect(await violations(html)).toStrictEqual([]);
  return html;
}

describe("violations", () => {
  // The first audit of the file, which loads axe: a fixed amount of work, which took 0.4
  // to 1.1 s plain on a loaded 10-core Mac, alone or beside the package's other files
  // (Vitest's default is 5 s). 60 s leaves a margin of more than 50 times the slowest
  // plain run.
  it("finds what axe rejects, so that an empty list proves something", async () => {
    expect(await violations('<ul><li>One</li></ul><p role="listitem" class="empty">Nothing yet</p>')).toStrictEqual([
      'aria-required-parent: <p role="listitem" class="empty">Nothing yet</p>',
    ]);
    expect(await violations("<ul><li>One</li></ul>")).toStrictEqual([]);
  }, 60_000);
});

describe("Board.astro", () => {
  it("renders steps with Done and In progress tags in a collapsed summary", async () => {
    const html = await render([
      card(1, "doing", {
        steps: [
          { title: "Install", done: true },
          { title: "Build", done: false },
          { title: "Ship", done: false },
        ],
      }),
    ]);
    expect(html).toMatch(/data-status="done"[\s\S]*Install[\s\S]*tag-done">Done</);
    expect(html).toMatch(/data-status="doing"[\s\S]*Build[\s\S]*tag-doing">In progress</);
    expect(html).toMatch(/data-status="todo"[\s\S]*Ship/);
    expect(html.match(/class="tag /g)).toHaveLength(2);
    // A space between a step's title and its tag, so that the item reads "Install Done", never "InstallDone".
    expect(html).toContain('<span class="step-title">Install</span> <span class="tag tag-done">Done</span>');
    expect(html).toContain('<span class="step-title">Build</span> <span class="tag tag-doing">In progress</span>');
    expect(html).toMatch(/<details class="steps-toggle"[^>]*>\s*<summary[^>]*>\s*Steps 1\/3[\s\S]*· Build[\s\S]*<\/summary>/);
    expect(html).not.toMatch(/<details[^>]*\bopen\b/);
  });

  it("renders no In progress tag for a card outside the In progress column", async () => {
    const html = await render([card(1, "next", { steps: [{ title: "Install", done: false }] })]);
    expect(html).toMatch(/data-status="todo"[\s\S]*Install/);
    expect(html).not.toContain("tag-doing");
    expect(html).not.toContain('class="current"');
  });

  it("renders columns, cards, description and milestone", async () => {
    const html = await render([card(1, "doing", { description: "The core", milestone: "M1" }), card(2, "done"), card(3, "next")]);
    const tallies = [...html.matchAll(/<span class="tally"[^>]*>([\s\S]*?)<\/span>\s*(?=<span class="tally"|<span class="muted"> · updated|<\/p>)/g)].map(([, inner = ""]) =>
      inner.replace(/<[^>]+>/g, "").replace(/\s+/g, " ").trim(),
    );
    expect(tallies).toStrictEqual(["0 Later", "· 1 Next", "· 1 In progress", "· 1 Done"]);
    expect(html).not.toContain(" / ");
    expect(html).toContain('datetime="2026-11-07"');
    for (const title of ["In progress", "Next", "Later", "Done"]) {
      expect(html).toContain(title);
    }
    expect(html).toMatch(/data-id="1"[\s\S]*Card 1[\s\S]*The core[\s\S]*#1[\s\S]*M1/);
    expect(html).toContain('data-column="later"');
    expect(html).toContain("Nothing yet");
  });

  it("renders no empty date, description or milestone", async () => {
    const empty = await render([]);
    expect(empty).not.toContain("updated");
    expect(empty.match(/Nothing yet/g)).toHaveLength(4);
    // A real card with an empty description and milestone: neither is rendered.
    const html = await render([card(1, "next")]);
    const item = /<li class="card" data-id="1"[^>]*>([\s\S]*?)<\/li>/.exec(html)?.[1] ?? "";
    expect(item).toMatch(/<details class="card-toggle"[^>]*>\s*<summary[^>]*>\s*<h3[^>]*>Card 1<\/h3>\s*<\/summary>/);
    expect(item).not.toMatch(/<p[ >]/);
    expect(item).not.toContain('class="milestone"');
  });

  it("renders no step list or progress for a card without steps", async () => {
    const html = await render([card(1, "next")]);
    expect(html).toContain('data-id="1"');
    expect(html).not.toContain('class="steps"');
    expect(html).not.toContain("steps-toggle");
  });

  it("shows no current step in the summary when every step is done", async () => {
    const html = await render([card(1, "done", { steps: [{ title: "Install", done: true }] })]);
    expect(html).toMatch(/<summary[^>]*>\s*Steps 1\/1\s*<\/summary>/);
    expect(html).not.toContain('class="current"');
  });

  it("names each column heading with its count and unit, the unit for screen readers only", async () => {
    const html = await render([card(1, "doing"), card(2, "doing"), card(3, "next")]);
    const headings = [...html.matchAll(/<h2[^>]*>([\s\S]*?)<\/h2>/g)].map(([, inner = ""]) =>
      inner.replace(/<[^>]+>/g, "").replace(/\s+/g, " ").trim(),
    );
    expect(headings).toStrictEqual(["Later 0 cards", "Next 1 card", "In progress 2 cards", "Done 0 cards"]);
    expect(html).toMatch(/<span class="count"[^>]*>2<span class="visually-hidden"[^>]*> cards<\/span><\/span>/);
  });

  it("keeps list semantics on every column's card list, whatever its style", async () => {
    const html = await render([card(1, "doing"), card(2, "next"), card(3, "later"), card(4, "done")]);
    expect(html.match(/<ul\b[^>]*>/g)).toStrictEqual(Array.from({ length: 4 }, () => '<ul role="list">'));
  });

  it("collapses every card to its title, its content inside the same toggle", async () => {
    const html = await render([card(1, "doing", { description: "The core", milestone: "M1", steps: [{ title: "Install", done: false }] })]);
    expect(html).toMatch(
      /<li class="card" data-id="1"[^>]*>\s*<details class="card-toggle"[^>]*>\s*<summary[^>]*>\s*<h3[^>]*>Card 1<\/h3>\s*<span class="progress"[^>]*>\s*Steps 0\/1\s*<\/span>\s*<\/summary>[\s\S]*The core[\s\S]*steps-toggle[\s\S]*#1[\s\S]*M1[\s\S]*<\/details>\s*<\/li>/,
    );
    expect(html).not.toMatch(/<details[^>]*\bopen\b/);
  });

  it("shows an In progress card's progress on its closed line, and no other card's", async () => {
    const steps = [
      { title: "Install", done: true },
      { title: "Build", done: false },
      { title: "Ship", done: false },
    ];
    const html = await render([card(1, "doing", { steps }), card(2, "next", { steps }), card(3, "done", { steps }), card(4, "doing")]);
    expect(html.match(/class="progress"/g)).toHaveLength(1);
    expect(html).toMatch(/data-id="1"[^>]*>\s*<details class="card-toggle"[^>]*>\s*<summary[^>]*>\s*<h3[^>]*>Card 1<\/h3>\s*<span class="progress"[^>]*>\s*Steps 1\/3\s*<\/span>\s*<\/summary>/);
    expect(html).toMatch(/data-id="4"[^>]*>\s*<details class="card-toggle"[^>]*>\s*<summary[^>]*>\s*<h3[^>]*>Card 4<\/h3>\s*<\/summary>/);
  });

  it("shows the five most recent Done cards, the rest behind Show all", async () => {
    const done = ["01", "02", "03", "04", "05", "06", "07"].map((day, index) => card(index + 1, "done", { updatedAt: `2026-11-${day}` }));
    const html = await render([...done, card(8, "later")]);
    const column = /<section class="column" data-column="done"[\s\S]*?<\/section>/.exec(html)?.[0] ?? "";
    const [shown = "", more = ""] = column.split('<details class="more"');
    expect([...shown.matchAll(/data-id="(\d+)"/g)].map(([, id]) => id)).toStrictEqual(["7", "6", "5", "4", "3"]);
    expect([...more.matchAll(/data-id="(\d+)"/g)].map(([, id]) => id)).toStrictEqual(["2", "1"]);
    expect(more).toMatch(/^[^>]*>\s*<summary[^>]*>\s*<span class="when-closed"[^>]*>Show all 7<\/span>\s*<span class="when-open"[^>]*>Show fewer<\/span>\s*<\/summary>\s*<ul role="list">/);
    // The count in the heading is every card, hidden ones included.
    expect(column).toMatch(/<span class="count"[^>]*>7</);
    // No other column has a Show all.
    expect(html.match(/class="more"/g)).toHaveLength(1);
  });

  it("has no Show all when Done holds five cards or fewer", async () => {
    const html = await render(["01", "02", "03", "04", "05"].map((day, index) => card(index + 1, "done", { updatedAt: `2026-11-${day}` })));
    expect(html).not.toContain('class="more"');
    expect(html.match(/class="card"/g)).toHaveLength(5);
  });

  it("escapes card content (no HTML injection)", async () => {
    const html = await render([card(1, "next", { title: "<script>alert(1)</script>", description: "<img src=x onerror=1>" })]);
    expect(html).not.toContain("<script>alert(1)</script>");
    expect(html).not.toContain("<img src=x");
    expect(html).toContain("&lt;script&gt;");
  });
});
