import { describe, expect, it } from "vitest";
import type { Board, Card } from "../src/lib/board.ts";
import { DONE_SHOWN, toView } from "../src/lib/view.ts";

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
      cards: [{ ...card(1, "doing", "2026-11-01"), steps: [{ title: "A", done: true }] }],
    };
    expect(toView(allDone).columns[2]?.cards[0]?.currentStep).toBeNull();
    expect(toView({ ...allDone, cards: [card(1, "doing", "2026-11-01")] }).columns[2]?.cards[0]?.currentStep).toBeNull();
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

  it("groups cards by column, in classic kanban order, then in board order", () => {
    const cards = [
      card(1, "done", "2026-11-03"),
      card(2, "next", "2026-11-09"),
      card(3, "done", "2026-11-01"),
      card(4, "doing", "2026-11-05"),
      card(5, "later", "2026-11-02"),
    ];
    const board: Board = { version: 1, title: "P", nextId: 6, cards };
    /** A column whose cards are all shown. */
    const column = (id: string, title: string, list: unknown[]): unknown => ({ id, title, cards: list, shown: list, more: [] });
    expect(toView(board)).toEqual({
      title: "P",
      columns: [
        column("later", "Later", [plain(cards[4])]),
        column("next", "Next", [plain(cards[1])]),
        column("doing", "In progress", [plain(cards[3])]),
        column("done", "Done", [plain(cards[0]), plain(cards[2])]),
      ],
      lastUpdated: "2026-11-09",
    });
  });

  it("keeps board order outside Done, whatever the dates", () => {
    const board: Board = { version: 1, title: "P", nextId: 4, cards: [card(1, "next", "2026-11-01"), card(2, "next", "2026-11-09"), card(3, "next", "2026-11-05")] };
    expect(toView(board).columns[1]?.cards.map((item) => item.id)).toStrictEqual([1, 2, 3]);
  });

  it("orders Done most recent first, the later in the board first on the same day", () => {
    const board: Board = {
      version: 1,
      title: "P",
      nextId: 6,
      cards: [card(1, "done", "2026-11-02"), card(2, "done", "2026-11-09"), card(3, "done", "2026-11-02"), card(4, "done", "2026-10-30"), card(5, "done", "2026-11-09")],
    };
    expect(toView(board).columns[3]?.cards.map((item) => item.id)).toStrictEqual([5, 2, 3, 1, 4]);
  });

  it("shows the five most recent Done cards and puts the rest behind Show all", () => {
    // The limit is a fixed fact of the page: literal, so that a changed constant fails here.
    expect(DONE_SHOWN).toBe(5);
    const days = ["01", "02", "03", "04", "05", "06", "07"];
    const done = days.map((day, index) => card(index + 1, "done", `2026-11-${day}`));
    const board: Board = { version: 1, title: "P", nextId: 9, cards: [...done, card(8, "later", "2026-11-01")] };
    const [later, , , doneColumn] = toView(board).columns;
    expect(doneColumn?.cards.map((item) => item.id)).toStrictEqual([7, 6, 5, 4, 3, 2, 1]);
    expect(doneColumn?.shown.map((item) => item.id)).toStrictEqual([7, 6, 5, 4, 3]);
    expect(doneColumn?.more.map((item) => item.id)).toStrictEqual([2, 1]);
    // At exactly the limit, nothing is hidden; other columns never hide anything.
    const five: Board = { version: 1, title: "P", nextId: 6, cards: done.slice(0, 5) };
    expect(toView(five).columns[3]?.more).toStrictEqual([]);
    expect(toView(five).columns[3]?.shown).toHaveLength(5);
    const six: Board = { version: 1, title: "P", nextId: 7, cards: done.slice(0, 6) };
    expect(toView(six).columns[3]?.more.map((item) => item.id)).toStrictEqual([1]);
    const many: Board = { version: 1, title: "P", nextId: 9, cards: days.map((day, index) => card(index + 1, "later", `2026-11-${day}`)) };
    expect(toView(many).columns[0]?.shown).toHaveLength(7);
    expect(toView(many).columns[0]?.more).toStrictEqual([]);
    expect(later?.shown.map((item) => item.id)).toStrictEqual([8]);
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
    const view = toView({ version: 1, title: "P", nextId: 1, cards: [] });
    expect(view.lastUpdated).toBeNull();
    expect(view.columns.map((column) => [column.cards, column.shown, column.more])).toStrictEqual(Array.from({ length: 4 }, () => [[], [], []]));
  });
});
