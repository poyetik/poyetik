import { describe, expect, it } from "vitest";
import type { Board, Card } from "../src/lib/board.ts";
import { toView } from "../src/lib/view.ts";

function card(id: number, column: Card["column"], updatedAt: string): Card {
  return { id: `R-${String(id)}`, title: `C${String(id)}`, description: "", column, milestone: "", createdAt: "2026-01-01", updatedAt };
}

describe("toView", () => {
  it("groups cards by column in board order", () => {
    const cards = [
      card(1, "done", "2026-11-03"),
      card(2, "next", "2026-11-09"),
      card(3, "done", "2026-11-01"),
      card(4, "doing", "2026-11-05"),
      card(5, "later", "2026-11-02"),
    ];
    const board: Board = { version: 1, title: "P", nextId: 6, cards };
    expect(toView(board)).toEqual({
      title: "P",
      columns: [
        { id: "later", title: "Later", cards: [cards[4]] },
        { id: "next", title: "Next", cards: [cards[1]] },
        { id: "doing", title: "In progress", cards: [cards[3]] },
        { id: "done", title: "Done", cards: [cards[0], cards[2]] },
      ],
      total: 5,
      done: 2,
      lastUpdated: "2026-11-09",
    });
  });

  it("takes the most recent date even when it is not last", () => {
    const board: Board = {
      version: 1,
      title: "P",
      nextId: 3,
      cards: [card(1, "later", "2026-12-01"), card(2, "later", "2026-02-01")],
    };
    expect(toView(board).lastUpdated).toBe("2026-12-01");
  });

  it("handles an empty board", () => {
    expect(toView({ version: 1, title: "P", nextId: 1, cards: [] })).toMatchObject({ total: 0, done: 0, lastUpdated: null });
  });
});
