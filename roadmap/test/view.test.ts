import { describe, expect, it } from "vitest";
import type { Board, Card } from "../src/lib/board.ts";
import { toView } from "../src/lib/view.ts";

function card(id: number, column: Card["column"], updatedAt: string): Card {
  return { id, title: `C${String(id)}`, description: "", column, milestone: "", createdAt: "2026-01-01", updatedAt, tasks: [] };
}

function plain(c: Card | undefined): unknown {
  return { ...c, steps: [], doneSteps: 0, currentStep: null };
}

describe("toView", () => {
  it("adds step statuses and the done count to each card", () => {
    const tasks = [
      { title: "A", done: true },
      { title: "B", done: false },
      { title: "C", done: true },
      { title: "D", done: false },
    ];
    const board: Board = { version: 1, title: "P", nextId: 2, cards: [{ ...card(1, "doing", "2026-11-01"), tasks }] };
    const view = toView(board).columns[2]?.cards[0];
    expect(view?.steps).toEqual([
      { title: "A", done: true, status: "done" },
      { title: "B", done: false, status: "doing" },
      { title: "C", done: true, status: "done" },
      { title: "D", done: false, status: "todo" },
    ]);
    expect(view?.doneSteps).toBe(2);
    expect(view?.currentStep).toBe("B");
  });

  it("has no current step when every step is done or there is none", () => {
    const allDone: Board = {
      version: 1,
      title: "P",
      nextId: 2,
      cards: [{ ...card(1, "doing", "2026-11-01"), tasks: [{ title: "A", done: true }] }],
    };
    expect(toView(allDone).columns[2]?.cards[0]?.currentStep).toBeNull();
    expect(toView({ ...allDone, cards: [card(1, "doing", "2026-11-01")] }).columns[2]?.cards[0]?.currentStep).toBeNull();
  });

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
        { id: "later", title: "Later", cards: [plain(cards[4])] },
        { id: "next", title: "Next", cards: [plain(cards[1])] },
        { id: "doing", title: "In progress", cards: [plain(cards[3])] },
        { id: "done", title: "Done", cards: [plain(cards[0]), plain(cards[2])] },
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
