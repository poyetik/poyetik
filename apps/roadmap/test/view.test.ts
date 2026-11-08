import { describe, expect, it } from "vitest";
import type { Board, Card } from "../src/lib/board.ts";
import { toView, DISPLAY_RANK } from "../src/lib/view.ts";

/** Builds a card with id `id` in `column`, last updated `updatedAt`, and no steps. */
function card(id: number, column: Card["column"], updatedAt: string): Card {
  return { id, title: `C${String(id)}`, description: "", column, milestone: "", createdAt: "2026-01-01", updatedAt, steps: [] };
}

/** The view expected for a card without steps: the card plus empty step fields. */
function plain(source: Card | undefined): unknown {
  return { ...source, steps: [], doneSteps: 0, currentStep: null };
}

describe("toView", () => {
  it("adds step statuses and the done count to each card", () => {
    const steps = [
      { title: "A", done: true },
      { title: "B", done: false },
      { title: "C", done: true },
      { title: "D", done: false },
    ];
    const board: Board = { version: 1, title: "P", nextId: 2, cards: [{ ...card(1, "doing", "2026-11-01"), steps }] };
    const view = toView(board).columns[0]?.cards[0];
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
      cards: [{ ...card(1, "doing", "2026-11-01"), steps: [{ title: "A", done: true }] }],
    };
    expect(toView(allDone).columns[0]?.cards[0]?.currentStep).toBeNull();
    expect(toView({ ...allDone, cards: [card(1, "doing", "2026-11-01")] }).columns[0]?.cards[0]?.currentStep).toBeNull();
  });

  it("has no current step outside the In progress column", () => {
    const steps = [
      { title: "A", done: true },
      { title: "B", done: false },
    ];
    const waiting: Board = { version: 1, title: "P", nextId: 2, cards: [{ ...card(1, "next", "2026-11-01"), steps }] };
    const view = toView(waiting).columns[1]?.cards[0];
    expect(view?.currentStep).toBeNull();
    expect(view?.steps.map((step) => step.status)).toEqual(["done", "todo"]);
    expect(view?.doneSteps).toBe(1);
  });

  it("groups cards by column, in progress first, then in board order", () => {
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
        { id: "doing", title: "In progress", cards: [plain(cards[3])] },
        { id: "next", title: "Next", cards: [plain(cards[1])] },
        { id: "later", title: "Later", cards: [plain(cards[4])] },
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

describe("DISPLAY_RANK", () => {
  it("shows the work in progress first and what is done last", () => {
    expect(DISPLAY_RANK).toStrictEqual({ doing: 0, next: 1, later: 2, done: 3 });
  });
});
