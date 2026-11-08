import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  addCard,
  addStep,
  completeStep,
  findCard,
  ArgumentError,
  BoardError,
  COLUMNS,
  editCard,
  emptyBoard,
  formatId,
  setTitle,
  isColumnId,
  moveCard,
  parseBoard,
  parseId,
  removeCard,
  removeStep,
  renameStep,
  reopenStep,
  parseStepNumber,
  withStatus,
  serializeBoard,
  type Board,
  type Card,
} from "../src/lib/board.ts";

/** Date of the fixtures' creation and last update. */
const DAY = "2026-11-07";
/** A later day passed as `today`, so a test can tell an updated date from an untouched one. */
const LATER = "2026-11-08";

/** Builds a valid card with id `id` in `column` and no steps, dated `DAY`; `extra` overrides any field. */
function card(id: number, column: Card["column"], extra: Partial<Card> = {}): Card {
  return {
    id,
    title: `Card ${String(id)}`,
    description: "",
    column,
    milestone: "",
    createdAt: DAY,
    updatedAt: DAY,
    steps: [],
    ...extra,
  };
}

/** Builds a valid board holding `cards`, with `nextId` just past them; the cards' ids must be 1 to n. */
function board(cards: Card[]): Board {
  return { version: 1, title: "Project", nextId: cards.length + 1, cards };
}

/** A card as written in board.json: its steps under the file's key `tasks` (see GLOSSARY.md). */
function stored(source: Card): Record<string, unknown> {
  const { steps, ...rest } = source;
  return { ...rest, tasks: steps };
}

/** Builds a valid untyped board, as read from JSON, with one card; `overrides` replaces top-level fields to make it invalid in one precise way. */
function raw(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return { version: 1, title: "Project", nextId: 2, cards: [stored(card(1, "next"))], ...overrides };
}

/** Builds an untyped board whose only card has the fields `overrides` replaced, to make that card invalid in one precise way. */
function rawCard(overrides: Record<string, unknown>): Record<string, unknown> {
  return raw({ cards: [{ ...stored(card(1, "next")), ...overrides }] });
}

/** Asserts that `fn` throws a `BoardError` with exactly `message`. */
function expectBoardError(fn: () => unknown, message: string): void {
  expect(fn).toThrow(BoardError);
  expect(fn).toThrow(new BoardError(message));
}

/** Asserts that `fn` refuses a value typed by a user, with `message`. */
function expectArgumentError(fn: () => unknown, message: string): void {
  expect(fn).toThrow(ArgumentError);
  expect(fn).toThrow(new ArgumentError(message));
}

describe("ArgumentError", () => {
  it("is a named BoardError: a usage error of the command line", () => {
    const error = new ArgumentError("bad");
    expect(error).toBeInstanceOf(BoardError);
    expect(error.name).toBe("ArgumentError");
  });
});

describe("BoardError", () => {
  it("is a named Error", () => {
    const error = new BoardError("boom");
    expect(error).toBeInstanceOf(Error);
    expect(error.name).toBe("BoardError");
    expect(error.message).toBe("boom");
  });
});

describe("COLUMNS", () => {
  it("defines the four columns in data and CLI order", () => {
    expect(COLUMNS).toEqual([
      { id: "later", title: "Later" },
      { id: "next", title: "Next" },
      { id: "doing", title: "In progress" },
      { id: "done", title: "Done" },
    ]);
  });

  it("recognizes only column ids", () => {
    for (const column of COLUMNS) {
      expect(isColumnId(column.id)).toBe(true);
    }
    for (const value of ["", "Later", "todo", 1, null, undefined]) {
      expect(isColumnId(value)).toBe(false);
    }
  });
});

describe("card ids", () => {
  it("formats an id with a hash", () => {
    expect(formatId(1)).toBe("#1");
    expect(formatId(123)).toBe("#123");
  });

  it("parses an id with or without a hash", () => {
    expect(parseId("1")).toBe(1);
    expect(parseId("#1")).toBe(1);
    expect(parseId("#120")).toBe(120);
    expect(parseId("907")).toBe(907);
  });

  it("parses the largest safe integer, and nothing beyond it", () => {
    expect(parseId(String(Number.MAX_SAFE_INTEGER))).toBe(Number.MAX_SAFE_INTEGER);
    // 2^53 and above read back as another number: two typed ids could name the same card.
    expectArgumentError(() => parseId("9007199254740992"), "invalid id: 9007199254740992");
    expectArgumentError(() => parseId("#99999999999999999999"), "invalid id: #99999999999999999999");
  });

  it.each(["", "#", "0", "#0", "01", "-1", "1a", "a1", "##1", " 1", "1 ", "R-1", "1.5"])(
    "rejects an invalid id: %j",
    (text) => {
      expectArgumentError(() => parseId(text), `invalid id: ${text}`);
    },
  );
});

describe("emptyBoard", () => {
  it("creates an empty board with a trimmed title", () => {
    expect(emptyBoard("  poyetik  ")).toEqual({ version: 1, title: "poyetik", nextId: 1, cards: [] });
  });

  it("rejects an empty title", () => {
    expectBoardError(() => emptyBoard("   "), "title must be a non-empty string");
  });
});

describe("parseBoard", () => {
  it("accepts a valid board and trims text", () => {
    const parsed = parseBoard(rawCard({ title: "  T  ", description: " d ", milestone: " m " }));
    expect(parsed).toEqual({
      version: 1,
      title: "Project",
      nextId: 2,
      cards: [{ ...card(1, "next"), title: "T", description: "d", milestone: "m" }],
    });
  });

  it("accepts a board with no cards", () => {
    expect(parseBoard(raw({ cards: [], nextId: 1 }))).toEqual({ version: 1, title: "Project", nextId: 1, cards: [] });
  });

  it("trims the board title", () => {
    expect(parseBoard(raw({ title: " P " })).title).toBe("P");
  });

  it.each([null, [], "x", 3])("rejects a board that is not an object: %j", (value) => {
    expectBoardError(() => parseBoard(value), "board must be an object");
  });

  it("rejects missing or extra fields", () => {
    expectBoardError(
      () => parseBoard({ version: 1, title: "P", cards: [] }),
      "board: expected fields cards, nextId, title, version, got cards, title, version",
    );
    expectBoardError(
      () => parseBoard({ ...raw(), extra: true }),
      "board: expected fields cards, nextId, title, version, got cards, extra, nextId, title, version",
    );
  });

  it("rejects another version", () => {
    expectBoardError(() => parseBoard(raw({ version: 2 })), "unsupported version: 2");
  });

  it.each([0, -1, 1.5, "2", null, true, [2], 2 ** 53, Number.MAX_VALUE, Infinity])("rejects an invalid nextId: %j", (nextId) => {
    expectBoardError(() => parseBoard(raw({ nextId, cards: [] })), "nextId must be a safe integer >= 1");
  });

  it("accepts the largest safe nextId", () => {
    expect(parseBoard(raw({ nextId: Number.MAX_SAFE_INTEGER, cards: [] })).nextId).toBe(Number.MAX_SAFE_INTEGER);
  });

  it("rejects cards that is not an array", () => {
    expectBoardError(() => parseBoard(raw({ cards: {} })), "cards must be an array");
  });

  it("rejects an empty or non-string board title", () => {
    expectBoardError(() => parseBoard(raw({ title: " " })), "title must be a non-empty string");
    expectBoardError(() => parseBoard(raw({ title: 3 })), "title must be a non-empty string");
  });

  it("locates an invalid card by its id and its 1-based entry, since ids are not in file order", () => {
    const cards = [stored(card(2, "next")), { ...stored(card(1, "next")), title: " " }];
    expectBoardError(() => parseBoard(raw({ nextId: 3, cards })), "card #1 (entry 2) title must be a non-empty string");
  });

  it("rejects a card that is not an object", () => {
    expectBoardError(() => parseBoard(raw({ cards: [stored(card(1, "next")), "x"], nextId: 3 })), "card entry 2 must be an object");
    expectBoardError(() => parseBoard(raw({ cards: [null] })), "card entry 1 must be an object");
    expectBoardError(() => parseBoard(raw({ cards: [[]] })), "card entry 1 must be an object");
  });

  it("rejects a card with missing or extra fields", () => {
    const withoutMilestone = Object.fromEntries(Object.entries(stored(card(1, "next"))).filter(([key]) => key !== "milestone"));
    expectBoardError(
      () => parseBoard(raw({ cards: [withoutMilestone] })),
      "card entry 1: expected fields column, createdAt, description, id, milestone, tasks, title, updatedAt, got column, createdAt, description, id, tasks, title, updatedAt",
    );
  });

  it.each([0, -1, 1.5, "1", "#1", null, true, [1], 2 ** 53])("rejects an invalid id: %j", (id) => {
    expectBoardError(() => parseBoard(rawCard({ id })), "card entry 1: invalid id");
  });

  it("accepts multi-digit ids", () => {
    expect(parseBoard(raw({ nextId: 124, cards: [stored(card(123, "next"))] })).cards[0]?.id).toBe(123);
  });

  it("rejects an empty card title", () => {
    expectBoardError(() => parseBoard(rawCard({ title: "  " })), "card #1 (entry 1) title must be a non-empty string");
  });

  it("rejects a non-string description or milestone", () => {
    expectBoardError(() => parseBoard(rawCard({ description: 1 })), "card #1 (entry 1) description must be a string");
    expectBoardError(() => parseBoard(rawCard({ milestone: null })), "card #1 (entry 1) milestone must be a string");
  });

  it("rejects a control, bidirectional or line separator character in every text field", () => {
    // Checked before trimming: a trailing newline is refused, not silently dropped.
    expectBoardError(() => parseBoard(raw({ title: "Project\u001b[2J" })), "title must not hold the control or formatting character \\u{1b}");
    expectBoardError(() => parseBoard(raw({ title: "Project\n" })), "title must not hold the control or formatting character \\u{a}");
    expectBoardError(() => parseBoard(rawCard({ title: "a\u202eb" })), "card #1 (entry 1) title must not hold the control or formatting character \\u{202e}");
    expectBoardError(() => parseBoard(rawCard({ description: "line\u2028line" })), "card #1 (entry 1) description must not hold the control or formatting character \\u{2028}");
    expectBoardError(() => parseBoard(rawCard({ milestone: "M1\u0085" })), "card #1 (entry 1) milestone must not hold the control or formatting character \\u{85}");
    expectBoardError(
      () => parseBoard(rawCard({ tasks: [{ title: "\u2066step", done: false }] })),
      "card #1 (entry 1) step 1 title must not hold the control or formatting character \\u{2066}",
    );
  });

  it("keeps other text, the neighbors of the refused ranges included", () => {
    const text = "Engine é 音 \u00a0\u202f\u206a";
    const parsed = parseBoard(rawCard({ title: text, description: text, milestone: text, tasks: [{ title: text, done: false }] }));
    expect(parsed.cards[0]).toMatchObject({ title: text, description: text, milestone: text, steps: [{ title: text, done: false }] });
  });

  it("rejects an unknown column", () => {
    expectBoardError(
      () => parseBoard(rawCard({ column: "todo" })),
      "unknown column: todo (expected: later, next, doing, done)",
    );
  });

  it.each([
    "2026-11-7",
    "2026-1-07",
    "226-11-07",
    "02026-11-07",
    "2026-11-07x",
    "x2026-11-07",
    "2026/11/07",
    "2026-11-007",
    "2026-011-07",
    "aaaa-bb-cc",
    // Truncated or padded forms: a prefix of a valid date is not a date.
    "2026",
    "2026-11",
    "2026-11-0",
    " 2026-11-08",
    "2026-11-08 ",
    20261107,
    true,
    null,
    ["2026-11-07"],
  ])("rejects an invalid date: %j", (createdAt) => {
    expectBoardError(() => parseBoard(rawCard({ createdAt })), "card #1 (entry 1) createdAt must be a YYYY-MM-DD date");
  });

  it("rejects an invalid update date", () => {
    expectBoardError(() => parseBoard(rawCard({ updatedAt: "yesterday" })), "card #1 (entry 1) updatedAt must be a YYYY-MM-DD date");
    // Well formed, but not a day of the calendar.
    for (const impossible of ["2026-02-31", "2026-02-29", "2026-13-01", "2026-00-10", "2026-04-31", "2026-06-00", "2026-1-1", "x2026-01-01", "2026-01-01x", "+002026-01-01", ""]) {
      expectBoardError(() => parseBoard(rawCard({ updatedAt: impossible })), "card #1 (entry 1) updatedAt must be a YYYY-MM-DD date");
    }
    expect(parseBoard(rawCard({ updatedAt: "2028-02-29" })).cards[0]?.updatedAt).toBe("2028-02-29");
  });

  it.each(["0001-01-01", "2024-02-29", "2000-02-29", "2026-01-31", "2026-04-30", "2026-12-31", "9999-12-31"])(
    "accepts the calendar day %s and writes it back unchanged",
    (day) => {
      const parsed = parseBoard(rawCard({ createdAt: day, updatedAt: day }));
      expect(parsed.cards[0]).toMatchObject({ createdAt: day, updatedAt: day });
      expect(JSON.parse(serializeBoard(parsed))).toEqual(rawCard({ createdAt: day, updatedAt: day }));
    },
  );

  it("rejects duplicate ids", () => {
    expectBoardError(() => parseBoard(raw({ nextId: 3, cards: [stored(card(1, "next")), stored(card(1, "done"))] })), "duplicate id: #1");
  });

  it("rejects an id greater than or equal to nextId", () => {
    expectBoardError(() => parseBoard(raw({ nextId: 2, cards: [stored(card(2, "next"))] })), "#2 must be lower than nextId (2)");
    expectBoardError(() => parseBoard(raw({ nextId: 2, cards: [stored(card(5, "next"))] })), "#5 must be lower than nextId (2)");
  });
});

describe("serializeBoard", () => {
  it("produces stable, indented JSON with a trailing newline", () => {
    const sample = board([
      card(1, "next", { description: "d", milestone: "M1", steps: [{ title: "Step", done: true }] }),
    ]);
    expect(serializeBoard(sample)).toBe(`{
  "version": 1,
  "title": "Project",
  "nextId": 2,
  "cards": [
    {
      "id": 1,
      "title": "Card 1",
      "description": "d",
      "column": "next",
      "milestone": "M1",
      "createdAt": "${DAY}",
      "updatedAt": "${DAY}",
      "tasks": [
        {
          "title": "Step",
          "done": true
        }
      ]
    }
  ]
}
`);
  });

  it("orders keys regardless of the original order", () => {
    const shuffled: Board = {
      cards: [
        {
          steps: [{ done: false, title: "S" }],
          updatedAt: DAY,
          createdAt: DAY,
          milestone: "",
          column: "next",
          description: "",
          title: "Card 1",
          id: 1,
        },
      ],
      nextId: 2,
      title: "Project",
      version: 1,
    };
    expect(serializeBoard(shuffled)).toBe(serializeBoard(board([card(1, "next", { steps: [{ title: "S", done: false }] })])));
  });

  it("writes the real board.json back byte for byte", () => {
    const text = readFileSync(new URL("../board.json", import.meta.url), "utf8");
    expect(serializeBoard(parseBoard(JSON.parse(text)))).toBe(text);
  });

  it("writes a card's steps under the file's key tasks, never steps", () => {
    const written: unknown = JSON.parse(serializeBoard(board([card(1, "next", { steps: [{ title: "S", done: false }] })])));
    expect(written).toEqual(raw({ cards: [stored(card(1, "next", { steps: [{ title: "S", done: false }] }))] }));
    expect(serializeBoard(board([card(1, "next")]))).not.toContain('"steps"');
  });

  it("round-trips through parseBoard", () => {
    const original = board([card(1, "next"), card(2, "done", { milestone: "M" })]);
    expect(parseBoard(JSON.parse(serializeBoard(original)))).toEqual(original);
  });
});

describe("addCard", () => {
  it("adds a card with default values", () => {
    const result = addCard(emptyBoard("P"), { title: "  Engine  " }, DAY);
    expect(result.card).toEqual({
      id: 1,
      title: "Engine",
      description: "",
      column: "later",
      milestone: "",
      createdAt: DAY,
      updatedAt: DAY,
      steps: [],
    });
    expect(result.board).toEqual({ version: 1, title: "P", nextId: 2, cards: [result.card] });
  });

  it("appends with every field", () => {
    const start = board([card(1, "done")]);
    const result = addCard(start, { title: "T", description: " d ", column: "doing", milestone: " M1 " }, DAY);
    expect(result.card).toEqual({ ...card(2, "doing"), title: "T", description: "d", milestone: "M1" });
    expect(result.board.cards).toEqual([card(1, "done"), result.card]);
    expect(result.board.nextId).toBe(3);
  });

  it("does not mutate the original board", () => {
    const start = emptyBoard("P");
    addCard(start, { title: "T" }, DAY);
    expect(start).toEqual(emptyBoard("P"));
  });

  it("takes the last safe id, then refuses to add a card whose id could not be told apart", () => {
    const last = { ...emptyBoard("P"), nextId: Number.MAX_SAFE_INTEGER - 1 };
    const result = addCard(last, { title: "T" }, DAY);
    expect(result.card.id).toBe(Number.MAX_SAFE_INTEGER - 1);
    expect(result.board.nextId).toBe(Number.MAX_SAFE_INTEGER);
    expect(parseBoard(JSON.parse(serializeBoard(result.board)))).toEqual(result.board);
    expectBoardError(() => addCard(result.board, { title: "U" }, DAY), "no card id left");
  });

  it("validates title and date", () => {
    expectBoardError(() => addCard(emptyBoard("P"), { title: " " }, DAY), "title must be a non-empty string");
    expectBoardError(() => addCard(emptyBoard("P"), { title: "T" }, "tomorrow"), "today's date must be a YYYY-MM-DD date");
  });
});

describe("moveCard", () => {
  const start = board([card(1, "next"), card(2, "next"), card(3, "done"), card(4, "next")]);
  const ids = (result: Board): string[] => result.cards.map((entry) => `${String(entry.id)}:${entry.column}`);

  it("moves to the end of the column by default and updates the date", () => {
    const moved = moveCard(start, 1, "done", undefined, LATER);
    expect(ids(moved)).toEqual(["2:next", "3:done", "4:next", "1:done"]);
    expect(moved.cards.at(-1)).toEqual({ ...card(1, "done"), updatedAt: LATER });
  });

  it("inserts at a given position in the column", () => {
    expect(ids(moveCard(start, 3, "next", 0, LATER))).toEqual(["3:next", "1:next", "2:next", "4:next"]);
    expect(ids(moveCard(start, 3, "next", 1, LATER))).toEqual(["1:next", "3:next", "2:next", "4:next"]);
    expect(ids(moveCard(start, 3, "next", 2, LATER))).toEqual(["1:next", "2:next", "3:next", "4:next"]);
    expect(ids(moveCard(start, 3, "next", 3, LATER))).toEqual(["1:next", "2:next", "4:next", "3:next"]);
  });

  it("reorders within the same column", () => {
    expect(ids(moveCard(start, 4, "next", 0, LATER))).toEqual(["4:next", "1:next", "2:next", "3:done"]);
    expect(ids(moveCard(start, 1, "next", 2, LATER))).toEqual(["2:next", "3:done", "4:next", "1:next"]);
  });

  it("moves into an empty column", () => {
    expect(ids(moveCard(start, 2, "later", 0, LATER))).toEqual(["1:next", "3:done", "4:next", "2:later"]);
  });

  it("does not mutate the original board", () => {
    const copy = structuredClone(start);
    moveCard(start, 1, "done", undefined, LATER);
    expect(start).toEqual(copy);
  });

  it("rejects an out-of-range or non-integer position", () => {
    expectBoardError(() => moveCard(start, 3, "next", 4, LATER), "invalid position: 4 (0 to 3)");
    expectBoardError(() => moveCard(start, 3, "next", -1, LATER), "invalid position: -1 (0 to 3)");
    expectBoardError(() => moveCard(start, 3, "next", 0.5, LATER), "invalid position: 0.5 (0 to 3)");
  });

  it("rejects an unknown card and an invalid date", () => {
    expectBoardError(() => moveCard(start, 9, "done", undefined, LATER), "card not found: #9");
    expectBoardError(() => moveCard(start, 1, "done", undefined, "x"), "today's date must be a YYYY-MM-DD date");
  });
});

describe("editCard", () => {
  const start = board([card(1, "next", { description: "d", milestone: "M" }), card(2, "done")]);

  it("edits only the given fields", () => {
    expect(editCard(start, 1, { title: " New " }, LATER).cards).toEqual([
      { ...card(1, "next", { description: "d", milestone: "M" }), title: "New", updatedAt: LATER },
      card(2, "done"),
    ]);
    expect(editCard(start, 1, { description: " e " }, LATER).cards[0]).toEqual({
      ...card(1, "next", { milestone: "M" }),
      description: "e",
      updatedAt: LATER,
    });
    expect(editCard(start, 1, { milestone: "" }, LATER).cards[0]).toEqual({
      ...card(1, "next", { description: "d" }),
      updatedAt: LATER,
    });
    expect(editCard(start, 1, { milestone: " M9 " }, LATER).cards[0]?.milestone).toBe("M9");
  });

  it("edits every field at once", () => {
    expect(editCard(start, 2, { title: "T", description: "D", milestone: "M2" }, LATER).cards[1]).toEqual({
      ...card(2, "done"),
      title: "T",
      description: "D",
      milestone: "M2",
      updatedAt: LATER,
    });
  });

  it("rejects an empty edit, an unknown card, an empty title, an invalid date", () => {
    expectBoardError(() => editCard(start, 1, {}, LATER), "nothing to edit");
    expectBoardError(() => editCard(start, 9, { title: "T" }, LATER), "card not found: #9");
    expectBoardError(() => editCard(start, 1, { title: " " }, LATER), "title must be a non-empty string");
    expectBoardError(() => editCard(start, 1, { title: "T" }, "x"), "today's date must be a YYYY-MM-DD date");
  });
});

describe("setTitle", () => {
  it("sets the trimmed title and keeps everything else", () => {
    const start = board([card(1, "next")]);
    expect(setTitle(start, "  Poyetik ")).toEqual({ ...start, title: "Poyetik" });
    expect(start.title).toBe("Project");
  });

  it("rejects an empty title", () => {
    expectBoardError(() => setTitle(board([]), " "), "title must be a non-empty string");
  });
});

describe("removeCard", () => {
  it("removes the card without touching nextId", () => {
    const start = board([card(1, "next"), card(2, "done")]);
    expect(removeCard(start, 1)).toEqual({ ...start, cards: [card(2, "done")] });
  });

  it("rejects an unknown card", () => {
    expectBoardError(() => removeCard(board([]), 1), "card not found: #1");
  });
});

describe("steps in parseBoard", () => {
  it("accepts steps and trims their titles", () => {
    expect(parseBoard(rawCard({ tasks: [{ title: " A ", done: true }, { title: "B", done: false }] })).cards[0]?.steps).toEqual([
      { title: "A", done: true },
      { title: "B", done: false },
    ]);
  });

  it("reads steps only from the file's key tasks, and refuses a steps key", () => {
    const { tasks, ...rest } = stored(card(1, "next"));
    expectBoardError(
      () => parseBoard(raw({ cards: [{ ...rest, steps: tasks }] })),
      "card entry 1: expected fields column, createdAt, description, id, milestone, tasks, title, updatedAt, got column, createdAt, description, id, milestone, steps, title, updatedAt",
    );
  });

  it("rejects steps that are not an array", () => {
    expectBoardError(() => parseBoard(rawCard({ tasks: {} })), "card #1 (entry 1) tasks must be an array");
  });

  it("rejects a step that is not an object", () => {
    expectBoardError(() => parseBoard(rawCard({ tasks: ["x"] })), "card #1 (entry 1) step 1 must be an object");
    expectBoardError(() => parseBoard(rawCard({ tasks: [{ title: "A", done: true }, null] })), "card #1 (entry 1) step 2 must be an object");
    expectBoardError(() => parseBoard(rawCard({ tasks: [[]] })), "card #1 (entry 1) step 1 must be an object");
  });

  it("rejects a step with missing or extra fields", () => {
    expectBoardError(
      () => parseBoard(rawCard({ tasks: [{ title: "A" }] })),
      "card #1 (entry 1) step 1: expected fields done, title, got title",
    );
  });

  it("rejects a step whose done flag is not a boolean", () => {
    expectBoardError(() => parseBoard(rawCard({ tasks: [{ title: "A", done: "yes" }] })), "card #1 (entry 1) step 1 done must be a boolean");
  });

  it("rejects a step with an empty title", () => {
    expectBoardError(() => parseBoard(rawCard({ tasks: [{ title: " ", done: false }] })), "card #1 (entry 1) step 1 title must be a non-empty string");
  });
});

describe("withStatus", () => {
  const statuses = (column: Card["column"], done: boolean[]): string[] =>
    withStatus(card(1, column, { steps: done.map((isDone, i) => ({ title: String(i), done: isDone })) })).map((step) => step.status);

  it("marks done steps, the first unfinished step in progress, the rest to do, in the In progress column", () => {
    expect(statuses("doing", [])).toEqual([]);
    expect(statuses("doing", [false, false, false])).toEqual(["doing", "todo", "todo"]);
    expect(statuses("doing", [true, false, false])).toEqual(["done", "doing", "todo"]);
    expect(statuses("doing", [true, false, true, false])).toEqual(["done", "doing", "done", "todo"]);
    expect(statuses("doing", [true, true])).toEqual(["done", "done"]);
  });

  it.each(["later", "next", "done"] as const)("has no step in progress in the %s column", (column) => {
    expect(statuses(column, [false, false])).toEqual(["todo", "todo"]);
    expect(statuses(column, [true, false, true, false])).toEqual(["done", "todo", "done", "todo"]);
    expect(statuses(column, [true, true])).toEqual(["done", "done"]);
  });

  it("keeps the title and done flag", () => {
    expect(withStatus(card(1, "doing", { steps: [{ title: "A", done: false }] }))).toEqual([{ title: "A", done: false, status: "doing" }]);
  });
});

describe("parseStepNumber", () => {
  it("parses positive integers", () => {
    expect(parseStepNumber("1")).toBe(1);
    expect(parseStepNumber("12")).toBe(12);
  });

  it.each(["", "0", "01", "-1", "1a", "a1", "#1", " 1", "1.5"])("rejects an invalid step number: %j", (text) => {
    expectArgumentError(() => parseStepNumber(text), `invalid step number: ${text}`);
  });
});

describe("findCard", () => {
  it("returns the card or fails", () => {
    const twoCards = board([card(1, "next"), card(2, "done")]);
    expect(findCard(twoCards, 2)).toEqual(card(2, "done"));
    expectBoardError(() => findCard(twoCards, 3), "card not found: #3");
  });
});

describe("step commands", () => {
  const steps = (current: Board): string[] => (current.cards[0]?.steps ?? []).map((step) => `${step.title}:${String(step.done)}`);
  const start = board([card(1, "doing", { steps: [{ title: "A", done: false }, { title: "B", done: false }, { title: "C", done: false }] }), card(2, "next")]);

  it("adds a step at the end, trimmed, and updates the date", () => {
    const next = addStep(start, 1, " D ", LATER);
    expect(steps(next)).toEqual(["A:false", "B:false", "C:false", "D:false"]);
    expect(next.cards[0]?.updatedAt).toBe(LATER);
    expect(next.cards[1]).toEqual(card(2, "next"));
  });

  it("completes the step in progress by default", () => {
    const once = completeStep(start, 1, undefined, LATER);
    expect(steps(once)).toEqual(["A:true", "B:false", "C:false"]);
    expect(steps(completeStep(once, 1, undefined, LATER))).toEqual(["A:true", "B:true", "C:false"]);
    expect(once.cards[0]?.updatedAt).toBe(LATER);
  });

  it("completes a given step", () => {
    expect(steps(completeStep(start, 1, 3, LATER))).toEqual(["A:false", "B:false", "C:true"]);
  });

  it("fails when there is no step in progress", () => {
    expectBoardError(() => completeStep(board([card(1, "next")]), 1, undefined, LATER), "#1 has no step in progress");
    const allDone = completeStep(completeStep(completeStep(start, 1, 1, LATER), 1, 2, LATER), 1, 3, LATER);
    expectBoardError(() => completeStep(allDone, 1, undefined, LATER), "#1 has no step in progress");
  });

  it.each(["later", "next", "done"] as const)("has no default step outside the In progress column (%s)", (column) => {
    const waiting = board([{ ...card(1, column), steps: [{ title: "A", done: false }] }]);
    expectBoardError(() => completeStep(waiting, 1, undefined, LATER), "#1 has no step in progress");
    expect(completeStep(waiting, 1, 1, LATER).cards[0]?.steps).toEqual([{ title: "A", done: true }]);
  });

  it("reopens a step", () => {
    const allDone = completeStep(completeStep(start, 1, 1, LATER), 1, 2, LATER);
    expect(steps(reopenStep(allDone, 1, 1, LATER))).toEqual(["A:false", "B:true", "C:false"]);
  });

  it("renames a step", () => {
    expect(steps(renameStep(start, 1, 2, " B2 ", LATER))).toEqual(["A:false", "B2:false", "C:false"]);
    expectBoardError(() => renameStep(start, 1, 2, " ", LATER), "step title must be a non-empty string");
  });

  it("removes a step", () => {
    expect(steps(removeStep(start, 1, 2, LATER))).toEqual(["A:false", "C:false"]);
    expect(steps(removeStep(start, 1, 1, LATER))).toEqual(["B:false", "C:false"]);
    expect(steps(removeStep(start, 1, 3, LATER))).toEqual(["A:false", "B:false"]);
  });

  it("does not mutate the original board", () => {
    const copy = structuredClone(start);
    addStep(start, 1, "D", LATER);
    completeStep(start, 1, undefined, LATER);
    removeStep(start, 1, 1, LATER);
    expect(start).toEqual(copy);
  });

  it("rejects out-of-range or non-integer step numbers", () => {
    for (const stepNumber of [0, 4, -1, 1.5]) {
      expectBoardError(() => reopenStep(start, 1, stepNumber, LATER), `invalid step number for #1: ${String(stepNumber)} (1 to 3)`);
      expectBoardError(() => completeStep(start, 1, stepNumber, LATER), `invalid step number for #1: ${String(stepNumber)} (1 to 3)`);
      expectBoardError(() => renameStep(start, 1, stepNumber, "X", LATER), `invalid step number for #1: ${String(stepNumber)} (1 to 3)`);
      expectBoardError(() => removeStep(start, 1, stepNumber, LATER), `invalid step number for #1: ${String(stepNumber)} (1 to 3)`);
    }
    expectBoardError(() => reopenStep(board([card(1, "next")]), 1, 1, LATER), "invalid step number for #1: 1 (1 to 0)");
  });

  it("rejects an empty title, an unknown card and an invalid date", () => {
    expectBoardError(() => addStep(start, 1, "  ", LATER), "step title must be a non-empty string");
    expectBoardError(() => addStep(start, 9, "D", LATER), "card not found: #9");
    expectBoardError(() => addStep(start, 1, "D", "x"), "today's date must be a YYYY-MM-DD date");
  });
});

describe("closure law", () => {
  /** Boards at the edges of the domain: empty, the last safe id, every column, steps all done and none done. */
  const edges: Board[] = [
    emptyBoard("P"),
    { ...emptyBoard("P"), nextId: Number.MAX_SAFE_INTEGER - 1 },
    {
      version: 1,
      title: "P",
      nextId: Number.MAX_SAFE_INTEGER - 1,
      cards: [
        card(1, "later", { steps: [{ title: "A", done: false }] }),
        card(Number.MAX_SAFE_INTEGER - 2, "doing", { steps: [{ title: "A", done: true }, { title: "B", done: true }] }),
        card(3, "next", { description: "d", milestone: "M" }),
        card(4, "done", { steps: [{ title: "A", done: true }] }),
      ],
    },
  ];

  /** Every command applied to `start` in every way that succeeds on it. */
  function results(start: Board): Board[] {
    const all: Board[] = [addCard(start, { title: "New", column: "doing" }, LATER).board, setTitle(start, "Poyetik")];
    for (const existing of start.cards) {
      for (const column of COLUMNS) {
        all.push(moveCard(start, existing.id, column.id, 0, LATER));
      }
      all.push(editCard(start, existing.id, { title: "T", description: "", milestone: "" }, LATER));
      all.push(removeCard(start, existing.id));
      all.push(addStep(start, existing.id, "S", LATER));
      existing.steps.forEach((_step, i) => {
        all.push(completeStep(start, existing.id, i + 1, LATER));
        all.push(reopenStep(start, existing.id, i + 1, LATER));
        all.push(renameStep(start, existing.id, i + 1, "R", LATER));
        all.push(removeStep(start, existing.id, i + 1, LATER));
      });
    }
    return all;
  }

  it.each(edges.map((start, i) => [i, start] as const))("keeps edge board %i valid through every command", (_i, start) => {
    expect(parseBoard(JSON.parse(serializeBoard(start)))).toEqual(start);
    for (const result of results(start)) {
      expect(parseBoard(JSON.parse(serializeBoard(result)))).toEqual(result);
    }
  });
});
