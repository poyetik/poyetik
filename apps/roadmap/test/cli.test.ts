import { describe, expect, it } from "vitest";
import { emptyBoard, parseBoard, serializeBoard, type Board } from "../src/lib/board.ts";
import { HELP, localDate, run, type Io, ignoreClosedReader, UsageError } from "../src/cli/cli.ts";

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
      file: "board.json",
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
  return parseBoard(JSON.parse(f.file));
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
    for (const action of ["add", "done", "undo", "edit", "rm"]) {
      expect(HELP).toContain(`  task ${action} <id>`);
    }
    expect(HELP).toContain("Steps: in a card of the doing column, the first step that is not done is in progress.");
    expect(HELP).toContain("Columns: later (Later), next (Next), doing (In progress), done (Done)");
  });
});

describe("add", () => {
  it("adds a card, writes the file and confirms", () => {
    const f = fake();
    expect(run(["add", "Engine", "--column", "doing", "--milestone", "M1", "--desc", "The core"], f.io)).toBe(0);
    expect(f.out).toEqual(["added: #1  doing  Engine [M1]"]);
    expect(boardOf(f).cards).toEqual([
      { id: 1, title: "Engine", description: "The core", column: "doing", milestone: "M1", createdAt: DAY, updatedAt: DAY, tasks: [] },
    ]);
    expect(f.file.endsWith("}\n")).toBe(true);
  });

  it("defaults to later and prints no empty milestone", () => {
    const f = fake();
    expect(run(["add", "Editor"], f.io)).toBe(0);
    expect(f.out).toEqual(["added: #1  later  Editor"]);
  });

  it("rejects zero or several titles without writing", () => {
    for (const args of [["add"], ["add", "a", "b"]]) {
      const f = fake();
      expect(run(args, f.io)).toBe(2);
      expect(f.err).toEqual(["error: expected exactly one argument: title (see: roadmap help)"]);
      expect(f.writes).toBe(0);
    }
  });

  it("rejects an unknown column", () => {
    const f = fake();
    expect(run(["add", "T", "--column", "todo"], f.io)).toBe(2);
    expect(f.err).toEqual(["error: unknown column: todo (expected: later, next, doing, done) (see: roadmap help)"]);
  });
});

describe("list", () => {
  it("lists all cards or those of one column", () => {
    const f = seeded();
    expect(run(["list"], f.io)).toBe(0);
    expect(f.out).toEqual(["#1  doing  Engine [M1]", "#2  later  Editor"]);
    f.out.length = 0;
    expect(run(["list", "--column", "later"], f.io)).toBe(0);
    expect(f.out).toEqual(["#2  later  Editor"]);
    expect(f.writes).toBe(0);
  });

  it("reports an empty list", () => {
    const f = seeded();
    expect(run(["list", "--column", "done"], f.io)).toBe(0);
    expect(f.out).toEqual(["(no cards)"]);
  });

  it("rejects an unknown column", () => {
    const f = seeded();
    expect(run(["list", "--column", "x"], f.io)).toBe(2);
    expect(f.err).toEqual(["error: unknown column: x (expected: later, next, doing, done) (see: roadmap help)"]);
  });
});

describe("show", () => {
  it("shows a card with its description and dates", () => {
    const f = seeded();
    expect(run(["show", "#1"], f.io)).toBe(0);
    expect(f.out).toEqual(["#1  doing  Engine [M1]", "The core", `created ${DAY}, updated ${DAY}`]);
  });

  it("prints no empty description", () => {
    const f = seeded();
    expect(run(["show", "#2"], f.io)).toBe(0);
    expect(f.out).toEqual(["#2  later  Editor", `created ${DAY}, updated ${DAY}`]);
  });

  it("rejects an invalid id", () => {
    for (const command of ["show", "edit", "rm"]) {
      const f = seeded();
      expect(run([command, "abc", "--title", "x"].slice(0, command === "edit" ? 4 : 2), f.io)).toBe(2);
      expect(f.err).toEqual(["error: invalid id: abc (see: roadmap help)"]);
      expect(f.writes).toBe(0);
    }
    const f = seeded();
    expect(run(["move", "R-1", "done"], f.io)).toBe(2);
    expect(f.err).toEqual(["error: invalid id: R-1 (see: roadmap help)"]);
  });

  it("rejects an unknown card or a missing id", () => {
    const f = seeded();
    expect(run(["show", "#9"], f.io)).toBe(1);
    expect(run(["show"], f.io)).toBe(2);
    expect(f.err).toEqual(["error: card not found: #9", "error: expected exactly one argument: id (see: roadmap help)"]);
  });
});

describe("move", () => {
  it("moves a card to a position", () => {
    const f = seeded();
    expect(run(["move", "2", "doing", "--position", "0"], f.io)).toBe(0);
    expect(f.out).toEqual(["moved: #2 → doing"]);
    expect(boardOf(f).cards.map((c) => `${String(c.id)}:${c.column}`)).toEqual(["2:doing", "1:doing"]);
    expect(f.writes).toBe(1);
  });

  it("moves to the end of the column without a position", () => {
    const f = seeded();
    expect(run(["move", "1", "done"], f.io)).toBe(0);
    expect(boardOf(f).cards.map((c) => `${String(c.id)}:${c.column}`)).toEqual(["2:later", "1:done"]);
  });

  it("rejects missing or extra arguments", () => {
    for (const args of [["move"], ["move", "1"], ["move", "1", "done", "x"]]) {
      const f = seeded();
      expect(run(args, f.io)).toBe(2);
      expect(f.err).toEqual(["error: expected: move <id> <column> (see: roadmap help)"]);
      expect(f.writes).toBe(0);
    }
  });

  it("rejects an unknown column", () => {
    const f = seeded();
    expect(run(["move", "1", "x"], f.io)).toBe(2);
    expect(f.err).toEqual(["error: unknown column: x (expected: later, next, doing, done) (see: roadmap help)"]);
  });

  it.each(["-1", "1a", "a1", "", "1.5", " 1"])("rejects a non-numeric position: %j", (position) => {
    const f = seeded();
    expect(run(["move", "1", "done", `--position=${position}`], f.io)).toBe(2);
    expect(f.err).toEqual([`error: invalid position: ${position} (see: roadmap help)`]);
  });

  it("accepts multi-digit positions, then bounds them", () => {
    const f = seeded();
    expect(run(["move", "1", "done", "--position", "10"], f.io)).toBe(1);
    expect(f.err).toEqual(["error: invalid position: 10 (0 to 0)"]);
  });
});

describe("edit", () => {
  it("edits the given fields", () => {
    const f = seeded();
    expect(run(["edit", "#2", "--title", "Web editor", "--desc", "Patching", "--milestone", "M2"], f.io)).toBe(0);
    expect(f.out).toEqual(["edited: #2"]);
    expect(boardOf(f).cards[1]).toMatchObject({ title: "Web editor", description: "Patching", milestone: "M2" });
  });

  it("rejects an empty edit", () => {
    const f = seeded();
    expect(run(["edit", "#2"], f.io)).toBe(1);
    expect(f.err).toEqual(["error: nothing to edit"]);
    expect(f.writes).toBe(0);
  });

  it("rejects a missing id", () => {
    const f = seeded();
    expect(run(["edit", "--title", "x"], f.io)).toBe(2);
    expect(f.err).toEqual(["error: expected exactly one argument: id (see: roadmap help)"]);
  });
});

describe("rm", () => {
  it("removes a card", () => {
    const f = seeded();
    expect(run(["rm", "1"], f.io)).toBe(0);
    expect(f.out).toEqual(["removed: #1"]);
    expect(boardOf(f).cards.map((c) => c.id)).toEqual([2]);
  });

  it("rejects an unknown card", () => {
    const f = seeded();
    expect(run(["rm", "9"], f.io)).toBe(1);
    expect(f.err).toEqual(["error: card not found: #9"]);
  });

  it("rejects a missing id", () => {
    const f = seeded();
    expect(run(["rm"], f.io)).toBe(2);
    expect(f.err).toEqual(["error: expected exactly one argument: id (see: roadmap help)"]);
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

  it("includes the JSON error in the message", () => {
    const f = fake("");
    expect(run(["validate"], f.io)).toBe(1);
    expect(f.err).toEqual(["error: unreadable board.json: SyntaxError: Unexpected end of JSON input"]);
  });

  it("lets disk read failures propagate", () => {
    const f = fake();
    const io: Io = {
      ...f.io,
      read: () => {
        throw new Error("disk missing");
      },
    };
    expect(() => run(["validate"], io)).toThrow("disk missing");
    expect(f.err).toEqual([]);
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
    expect(run(["frobnicate"], f.io)).toBe(2);
    expect(f.err).toEqual(["error: unknown command: frobnicate (see: roadmap help)"]);
  });

  it("rejects an unknown option", () => {
    const f = fake();
    expect(run(["list", "--nope"], f.io)).toBe(2);
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

describe("task", () => {
  function withSteps(): Fake {
    const f = seeded();
    expect(run(["task", "add", "1", "Install"], f.io)).toBe(0);
    expect(run(["task", "add", "#1", "Build"], f.io)).toBe(0);
    expect(run(["task", "add", "1", "Ship"], f.io)).toBe(0);
    f.out.length = 0;
    f.writes = 0;
    return f;
  }

  it("adds steps and prints the card with statuses", () => {
    const f = seeded();
    expect(run(["task", "add", "1", "Install"], f.io)).toBe(0);
    expect(f.out).toEqual(["#1  doing  Engine [M1]  0/1", "The core", "  1. Install  [In progress]"]);
    expect(boardOf(f).cards[0]?.tasks).toEqual([{ title: "Install", done: false }]);
  });

  it("shows no step in progress for a card outside the doing column", () => {
    const f = seeded();
    expect(run(["task", "add", "2", "Sketch"], f.io)).toBe(0);
    expect(f.out).toEqual(["#2  later  Editor  0/1", "  1. Sketch"]);
    expect(run(["task", "done", "2"], f.io)).toBe(1);
    expect(f.err).toEqual(["error: #2 has no task in progress"]);
  });

  it("completes the step in progress and moves the tag to the next one", () => {
    const f = withSteps();
    expect(run(["task", "done", "1"], f.io)).toBe(0);
    expect(f.out).toEqual([
      "#1  doing  Engine [M1]  1/3",
      "The core",
      "  1. Install  [Done]",
      "  2. Build  [In progress]",
      "  3. Ship",
    ]);
    expect(f.writes).toBe(1);
  });

  it("completes, reopens, renames and removes a given step", () => {
    const f = withSteps();
    expect(run(["task", "done", "1", "3"], f.io)).toBe(0);
    expect(boardOf(f).cards[0]?.tasks.map((t) => t.done)).toEqual([false, false, true]);
    expect(run(["task", "undo", "1", "3"], f.io)).toBe(0);
    expect(boardOf(f).cards[0]?.tasks.map((t) => t.done)).toEqual([false, false, false]);
    expect(run(["task", "edit", "1", "2", "Compile"], f.io)).toBe(0);
    expect(run(["task", "rm", "1", "1"], f.io)).toBe(0);
    expect(boardOf(f).cards[0]?.tasks).toEqual([
      { title: "Compile", done: false },
      { title: "Ship", done: false },
    ]);
    expect(f.out.at(-1)).toBe("  2. Ship");
  });

  it("shows steps in show and progress in list", () => {
    const f = withSteps();
    expect(run(["task", "done", "1"], f.io)).toBe(0);
    f.out.length = 0;
    expect(run(["list"], f.io)).toBe(0);
    expect(f.out).toEqual(["#1  doing  Engine [M1]  1/3", "#2  later  Editor"]);
    f.out.length = 0;
    expect(run(["show", "1"], f.io)).toBe(0);
    expect(f.out).toEqual([
      "#1  doing  Engine [M1]  1/3",
      "The core",
      "  1. Install  [Done]",
      "  2. Build  [In progress]",
      "  3. Ship",
      `created ${DAY}, updated ${DAY}`,
    ]);
  });

  it.each([
    [["task"], "unknown task action: (none) (expected: add, done, undo, edit, rm)"],
    [["task", "close", "1"], "unknown task action: close (expected: add, done, undo, edit, rm)"],
    [["task", "toString", "1"], "unknown task action: toString (expected: add, done, undo, edit, rm)"],
  ])("rejects an unknown action: %j", (args, message) => {
    const f = withSteps();
    expect(run(args, f.io)).toBe(2);
    expect(f.err).toEqual([`error: ${message} (see: roadmap help)`]);
    expect(f.writes).toBe(0);
  });

  it.each([
    [["task", "add"], "task add <id> <title>"],
    [["task", "add", "1"], "task add <id> <title>"],
    [["task", "add", "1", "a", "b"], "task add <id> <title>"],
    [["task", "done"], "task done <id> [n]"],
    [["task", "done", "1", "2", "3"], "task done <id> [n]"],
    [["task", "undo", "1"], "task undo <id> <n>"],
    [["task", "undo", "1", "2", "3"], "task undo <id> <n>"],
    [["task", "edit", "1", "2"], "task edit <id> <n> <title>"],
    [["task", "edit", "1", "2", "a", "b"], "task edit <id> <n> <title>"],
    [["task", "rm", "1"], "task rm <id> <n>"],
    [["task", "rm", "1", "2", "3"], "task rm <id> <n>"],
  ])("rejects wrong arguments: %j", (args, usage) => {
    const f = withSteps();
    expect(run(args, f.io)).toBe(2);
    expect(f.err).toEqual([`error: expected: ${usage} (see: roadmap help)`]);
    expect(f.writes).toBe(0);
  });

  it("rejects invalid ids and step numbers", () => {
    const f = withSteps();
    expect(run(["task", "add", "x", "T"], f.io)).toBe(2);
    expect(run(["task", "done", "1", "x"], f.io)).toBe(2);
    expect(run(["task", "undo", "x", "1"], f.io)).toBe(2);
    expect(run(["task", "edit", "1", "0", "T"], f.io)).toBe(2);
    expect(run(["task", "rm", "1", "9"], f.io)).toBe(1);
    expect(f.err).toEqual([
      "error: invalid id: x (see: roadmap help)",
      "error: invalid task number: x (see: roadmap help)",
      "error: invalid id: x (see: roadmap help)",
      "error: invalid task number: 0 (see: roadmap help)",
      "error: invalid task number for #1: 9 (1 to 3)",
    ]);
    expect(f.writes).toBe(0);
  });
});

describe("ignoreClosedReader", () => {
  /** A stream that records its error listener. */
  function stream(): { on: (event: "error", listener: (error: Error) => void) => unknown; emit: (error: Error) => void } {
    const listeners: ((error: Error) => void)[] = [];
    return {
      on: (event: string, listener: (error: Error) => void) => (event === "error" ? listeners.push(listener) : 0),
      emit: (error) => {
        listeners.forEach((listener) => {
          listener(error);
        });
      },
    };
  }

  it("drops a closed reader and lets any other stream error through", () => {
    const output = stream();
    ignoreClosedReader(output);
    expect(() => {
      output.emit(Object.assign(new Error("write EPIPE"), { code: "EPIPE" }));
    }).not.toThrow();
    const full = Object.assign(new Error("no space"), { code: "ENOSPC" });
    expect(() => {
      output.emit(full);
    }).toThrow(full);
    const plain = new Error("broken");
    expect(() => {
      output.emit(plain);
    }).toThrow(plain);
  });
});

describe("UsageError", () => {
  it("is a named Error", () => {
    expect(new UsageError("x")).toMatchObject({ name: "UsageError", message: "x" });
  });
});
