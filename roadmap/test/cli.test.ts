import { describe, expect, it } from "vitest";
import { emptyBoard, serializeBoard, type Board } from "../src/lib/board.ts";
import { HELP, localDate, run, type Io } from "../src/cli/cli.ts";

const DAY = "2026-11-07";

interface Fake {
  readonly io: Io;
  readonly out: string[];
  readonly err: string[];
  file: string;
  writes: number;
}

function fake(initial: string = serializeBoard(emptyBoard("Project"))): Fake {
  const state: Fake = {
    out: [],
    err: [],
    file: initial,
    writes: 0,
    io: {
      read: () => state.file,
      write: (text) => {
        state.file = text;
        state.writes += 1;
      },
      out: (line) => state.out.push(line),
      err: (line) => state.err.push(line),
      today: () => DAY,
    },
  };
  return state;
}

function boardOf(f: Fake): Board {
  return JSON.parse(f.file) as Board;
}

function seeded(): Fake {
  const f = fake();
  expect(run(["add", "Engine", "--column", "doing", "--milestone", "M1", "--desc", "The core"], f.io)).toBe(0);
  expect(run(["add", "Editor"], f.io)).toBe(0);
  f.out.length = 0;
  f.writes = 0;
  return f;
}

describe("localDate", () => {
  it("formats the local date with zero padding", () => {
    expect(localDate(new Date(2026, 0, 5, 23, 59))).toBe("2026-01-05");
    expect(localDate(new Date(2026, 10, 17))).toBe("2026-11-17");
  });
});

describe("help", () => {
  it("prints help with no command or with help", () => {
    for (const args of [[], ["help"]]) {
      const f = fake();
      expect(run(args, f.io)).toBe(0);
      expect(f.out).toEqual([HELP]);
    }
  });

  it("lists every command and column", () => {
    for (const word of ["list", "show", "add", "move", "edit", "rm", "validate", "help"]) {
      expect(HELP).toContain(`  ${word}`);
    }
    expect(HELP).toContain("Columns: later (Later), next (Next), doing (In progress), done (Done)");
  });
});

describe("add", () => {
  it("adds a card, writes the file and confirms", () => {
    const f = fake();
    expect(run(["add", "Engine", "--column", "doing", "--milestone", "M1", "--desc", "The core"], f.io)).toBe(0);
    expect(f.out).toEqual(["added: R-1  doing  Engine [M1]"]);
    expect(boardOf(f).cards).toEqual([
      { id: "R-1", title: "Engine", description: "The core", column: "doing", milestone: "M1", createdAt: DAY, updatedAt: DAY },
    ]);
    expect(f.file.endsWith("}\n")).toBe(true);
  });

  it("defaults to later and prints no empty milestone", () => {
    const f = fake();
    expect(run(["add", "Editor"], f.io)).toBe(0);
    expect(f.out).toEqual(["added: R-1  later  Editor"]);
  });

  it("rejects zero or several titles without writing", () => {
    for (const args of [["add"], ["add", "a", "b"]]) {
      const f = fake();
      expect(run(args, f.io)).toBe(1);
      expect(f.err).toEqual(["error: expected exactly one argument: title"]);
      expect(f.writes).toBe(0);
    }
  });

  it("rejects an unknown column", () => {
    const f = fake();
    expect(run(["add", "T", "--column", "todo"], f.io)).toBe(1);
    expect(f.err).toEqual(["error: unknown column: todo (expected: later, next, doing, done)"]);
  });
});

describe("list", () => {
  it("lists all cards or those of one column", () => {
    const f = seeded();
    expect(run(["list"], f.io)).toBe(0);
    expect(f.out).toEqual(["R-1  doing  Engine [M1]", "R-2  later  Editor"]);
    f.out.length = 0;
    expect(run(["list", "--column", "later"], f.io)).toBe(0);
    expect(f.out).toEqual(["R-2  later  Editor"]);
    expect(f.writes).toBe(0);
  });

  it("reports an empty list", () => {
    const f = seeded();
    expect(run(["list", "--column", "done"], f.io)).toBe(0);
    expect(f.out).toEqual(["(no cards)"]);
  });

  it("rejects an unknown column", () => {
    const f = seeded();
    expect(run(["list", "--column", "x"], f.io)).toBe(1);
    expect(f.err).toEqual(["error: unknown column: x (expected: later, next, doing, done)"]);
  });
});

describe("show", () => {
  it("shows a card with its description and dates", () => {
    const f = seeded();
    expect(run(["show", "R-1"], f.io)).toBe(0);
    expect(f.out).toEqual(["R-1  doing  Engine [M1]", "The core", `created ${DAY}, updated ${DAY}`]);
  });

  it("prints no empty description", () => {
    const f = seeded();
    expect(run(["show", "R-2"], f.io)).toBe(0);
    expect(f.out).toEqual(["R-2  later  Editor", `created ${DAY}, updated ${DAY}`]);
  });

  it("rejects an unknown card or a missing id", () => {
    const f = seeded();
    expect(run(["show", "R-9"], f.io)).toBe(1);
    expect(run(["show"], f.io)).toBe(1);
    expect(f.err).toEqual(["error: card not found: R-9", "error: expected exactly one argument: id"]);
  });
});

describe("move", () => {
  it("moves a card to a position", () => {
    const f = seeded();
    expect(run(["move", "R-2", "doing", "--position", "0"], f.io)).toBe(0);
    expect(f.out).toEqual(["moved: R-2 → doing"]);
    expect(boardOf(f).cards.map((c) => `${c.id}:${c.column}`)).toEqual(["R-2:doing", "R-1:doing"]);
    expect(f.writes).toBe(1);
  });

  it("moves to the end of the column without a position", () => {
    const f = seeded();
    expect(run(["move", "R-1", "done"], f.io)).toBe(0);
    expect(boardOf(f).cards.map((c) => `${c.id}:${c.column}`)).toEqual(["R-2:later", "R-1:done"]);
  });

  it("rejects missing or extra arguments", () => {
    for (const args of [["move"], ["move", "R-1"], ["move", "R-1", "done", "x"]]) {
      const f = seeded();
      expect(run(args, f.io)).toBe(1);
      expect(f.err).toEqual(["error: expected: move <id> <column>"]);
      expect(f.writes).toBe(0);
    }
  });

  it("rejects an unknown column", () => {
    const f = seeded();
    expect(run(["move", "R-1", "x"], f.io)).toBe(1);
    expect(f.err).toEqual(["error: unknown column: x (expected: later, next, doing, done)"]);
  });

  it.each(["-1", "1a", "a1", "", "1.5", " 1"])("rejects a non-numeric position: %j", (position) => {
    const f = seeded();
    expect(run(["move", "R-1", "done", `--position=${position}`], f.io)).toBe(1);
    expect(f.err).toEqual([`error: invalid position: ${position}`]);
  });

  it("accepts multi-digit positions, then bounds them", () => {
    const f = seeded();
    expect(run(["move", "R-1", "done", "--position", "10"], f.io)).toBe(1);
    expect(f.err).toEqual(["error: invalid position: 10 (0 to 0)"]);
  });
});

describe("edit", () => {
  it("edits the given fields", () => {
    const f = seeded();
    expect(run(["edit", "R-2", "--title", "Web editor", "--desc", "Patching", "--milestone", "M2"], f.io)).toBe(0);
    expect(f.out).toEqual(["edited: R-2"]);
    expect(boardOf(f).cards[1]).toMatchObject({ title: "Web editor", description: "Patching", milestone: "M2" });
  });

  it("rejects an empty edit", () => {
    const f = seeded();
    expect(run(["edit", "R-2"], f.io)).toBe(1);
    expect(f.err).toEqual(["error: nothing to edit"]);
    expect(f.writes).toBe(0);
  });

  it("rejects a missing id", () => {
    const f = seeded();
    expect(run(["edit", "--title", "x"], f.io)).toBe(1);
    expect(f.err).toEqual(["error: expected exactly one argument: id"]);
  });
});

describe("rm", () => {
  it("removes a card", () => {
    const f = seeded();
    expect(run(["rm", "R-1"], f.io)).toBe(0);
    expect(f.out).toEqual(["removed: R-1"]);
    expect(boardOf(f).cards.map((c) => c.id)).toEqual(["R-2"]);
  });

  it("rejects an unknown card", () => {
    const f = seeded();
    expect(run(["rm", "R-9"], f.io)).toBe(1);
    expect(f.err).toEqual(["error: card not found: R-9"]);
  });

  it("rejects a missing id", () => {
    const f = seeded();
    expect(run(["rm"], f.io)).toBe(1);
    expect(f.err).toEqual(["error: expected exactly one argument: id"]);
  });
});

describe("validate", () => {
  it("confirms a valid board", () => {
    const f = seeded();
    expect(run(["validate"], f.io)).toBe(0);
    expect(f.out).toEqual(["valid board: 2 card(s)"]);
    expect(f.writes).toBe(0);
  });

  it("reports unreadable JSON", () => {
    const f = fake("{oops");
    expect(run(["validate"], f.io)).toBe(1);
    expect(f.err).toHaveLength(1);
    expect(f.err[0]).toMatch(/^error: unreadable board\.json: .+/);
  });

  it("reports a read failure that is not an Error", () => {
    const f = fake();
    const io: Io = {
      ...f.io,
      read: () => {
        // eslint-disable-next-line @typescript-eslint/only-throw-error -- simulating a non-Error value
        throw "disk missing";
      },
    };
    expect(run(["validate"], io)).toBe(1);
    expect(f.err).toEqual(["error: unreadable board.json: disk missing"]);
  });

  it("reports an invalid board", () => {
    const f = fake('{"version": 2}');
    expect(run(["validate"], f.io)).toBe(1);
    expect(f.err).toEqual(["error: board: expected fields cards, nextId, title, version, got version"]);
  });
});

describe("general errors", () => {
  it("rejects an unknown command", () => {
    const f = fake();
    expect(run(["frobnicate"], f.io)).toBe(1);
    expect(f.err).toEqual(["error: unknown command: frobnicate"]);
  });

  it("rejects an unknown option", () => {
    const f = fake();
    expect(run(["list", "--nope"], f.io)).toBe(1);
    expect(f.err).toHaveLength(1);
    expect(f.err[0]).toMatch(/^error: .*--nope/);
  });

  it("rethrows unexpected errors", () => {
    const f = fake();
    const io: Io = {
      ...f.io,
      today: () => {
        throw new RangeError("broken clock");
      },
    };
    expect(() => run(["add", "T"], io)).toThrow(RangeError);
  });

  it("rethrows a TypeError without a code", () => {
    const f = fake();
    const io: Io = {
      ...f.io,
      today: () => {
        throw new TypeError("bug");
      },
    };
    expect(() => run(["add", "T"], io)).toThrow("bug");
  });
});
