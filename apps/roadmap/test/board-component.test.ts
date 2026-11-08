import { experimental_AstroContainer as AstroContainer } from "astro/container";
import { describe, expect, it } from "vitest";
import BoardComponent from "../src/components/Board.astro";
import type { Card } from "../src/lib/board.ts";
import { toView } from "../src/lib/view.ts";

function card(id: number, column: Card["column"], extra: Partial<Card> = {}): Card {
  return {
    id,
    title: `Card ${String(id)}`,
    description: "",
    column,
    milestone: "",
    createdAt: "2026-11-07",
    updatedAt: "2026-11-07",
    tasks: [],
    ...extra,
  };
}

async function render(cards: Card[]): Promise<string> {
  const container = await AstroContainer.create();
  const view = toView({ version: 1, title: "P", nextId: cards.length + 1, cards });
  return container.renderToString(BoardComponent, { props: { view } });
}

describe("Board.astro", () => {
  it("renders steps with Done and In progress tags in a collapsed summary", async () => {
    const html = await render([
      card(1, "doing", {
        tasks: [
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
    expect(html).toMatch(/<details class="steps-toggle"[^>]*>\s*<summary[^>]*>\s*Steps 1\/3[\s\S]*· Build[\s\S]*<\/summary>/);
    expect(html).not.toMatch(/<details[^>]*\bopen\b/);
  });

  it("renders no In progress tag for a card outside the In progress column", async () => {
    const html = await render([card(1, "next", { tasks: [{ title: "Install", done: false }] })]);
    expect(html).toMatch(/data-status="todo"[\s\S]*Install/);
    expect(html).not.toContain("tag-doing");
    expect(html).not.toContain('class="current"');
  });

  it("renders columns, cards, description and milestone", async () => {
    const html = await render([card(1, "doing", { description: "The core", milestone: "M1" }), card(2, "done"), card(3, "next")]);
    expect(html).toContain("<strong>1</strong> / 3 done");
    expect(html).toContain('datetime="2026-11-07"');
    for (const title of ["Later", "Next", "In progress", "Done"]) {
      expect(html).toContain(title);
    }
    expect(html).toMatch(/data-id="1"[\s\S]*Card 1[\s\S]*The core[\s\S]*#1[\s\S]*M1/);
    expect(html).toContain('data-column="later"');
    expect(html).toContain("Nothing yet");
  });

  it("renders no empty date, description or milestone", async () => {
    const html = await render([]);
    expect(html).not.toContain("updated");
    expect(html).not.toContain('class="milestone"');
    expect(html.match(/Nothing yet/g)).toHaveLength(4);
  });

  it("renders no step list or progress for a card without steps", async () => {
    const html = await render([card(1, "next")]);
    expect(html).toContain('data-id="1"');
    expect(html).not.toContain('class="steps"');
    expect(html).not.toContain("<details");
  });

  it("shows no current step in the summary when every step is done", async () => {
    const html = await render([card(1, "done", { tasks: [{ title: "Install", done: true }] })]);
    expect(html).toMatch(/<summary[^>]*>\s*Steps 1\/1\s*<\/summary>/);
    expect(html).not.toContain('class="current"');
  });

  it("escapes card content (no HTML injection)", async () => {
    const html = await render([card(1, "next", { title: "<script>alert(1)</script>", description: "<img src=x onerror=1>" })]);
    expect(html).not.toContain("<script>alert(1)</script>");
    expect(html).not.toContain("<img src=x");
    expect(html).toContain("&lt;script&gt;");
  });
});
