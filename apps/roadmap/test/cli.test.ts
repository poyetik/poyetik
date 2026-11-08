import { afterEach, describe, expect, it } from "vitest";
import { emptyBoard, parseBoard, serializeBoard, type Board } from "../src/lib/board.ts";
import { TIME_ZONES, offsetOf } from "./matrix.ts";
import { HELP, exitAfterStreamFailure, failure, guardStreams, localDate, run, type ErrorSource, type Io, UsageError } from "../src/cli/cli.ts";

/** Today's date as the fake clock gives it. */
const DAY = "2026-11-07";

/** An in-memory `Io`: the board file held as text, the lines printed on each stream, and a count of writes, to check that failed commands write nothing. */
interface Fake {
  readonly io: Io;
  readonly out: string[];
  readonly err: string[];
  file: string;
  writes: number;
}

/** Builds a fake `Io` whose board file holds `initial` (an empty board by default) and whose clock always says `DAY`. */
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

/** Parses the board file currently held by the fake. */
function boardOf(cli: Fake): Board {
  return parseBoard(JSON.parse(cli.file));
}

/** Builds a fake holding two cards: #1 Engine (doing, milestone M1, with a description) and #2 Editor (later); output and write count are reset so tests see only their own command. */
function seeded(): Fake {
  const cli = fake();
  expect(run(["add", "Engine", "--column", "doing", "--milestone", "M1", "--desc", "The core"], cli.io)).toBe(0);
  expect(run(["add", "Editor"], cli.io)).toBe(0);
  cli.out.length = 0;
  cli.writes = 0;
  return cli;
}

describe("localDate", () => {
  const zone = process.env.TZ;

  afterEach(() => {
    if (zone === undefined) {
      delete process.env.TZ;
    } else {
      process.env.TZ = zone;
    }
  });

  // Node reads TZ again when it is assigned: each case runs in its own zone, whatever the machine's.
  it.each(TIME_ZONES)("formats the local day with zero padding in %s, early and late in the day", (timeZone) => {
    process.env.TZ = timeZone;
    expect(new Date(2026, 0, 5, 0, 30).getTimezoneOffset()).toBe(offsetOf(timeZone));
    expect(localDate(new Date(2026, 0, 5, 0, 30))).toBe("2026-01-05");
    expect(localDate(new Date(2026, 0, 5, 23, 30))).toBe("2026-01-05");
    expect(localDate(new Date(2026, 10, 17, 23, 59))).toBe("2026-11-17");
  });
});

describe("help", () => {
  it("prints help with no command or with help", () => {
    for (const args of [[], ["help"]]) {
      const cli = fake();
      expect(run(args, cli.io)).toBe(0);
      expect(cli.out).toEqual([HELP]);
    }
  });

  it("lists every command and column", () => {
    for (const word of ["list", "show", "add", "move", "edit", "rm", "title", "validate", "help"]) {
      expect(HELP).toContain(`  ${word}`);
    }
    for (const action of ["add", "done", "undo", "edit", "rm"]) {
      expect(HELP).toContain(`  step ${action} <id>`);
    }
    expect(HELP).toContain("Steps: in a card of the doing column, the first step that is not done is in progress.");
    expect(HELP).toContain("Columns: later (Later), next (Next), doing (In progress), done (Done)");
  });
});

describe("add", () => {
  it("adds a card, writes the file and confirms", () => {
    const cli = fake();
    expect(run(["add", "Engine", "--column", "doing", "--milestone", "M1", "--desc", "The core"], cli.io)).toBe(0);
    expect(cli.out).toEqual(["added: #1  doing  Engine [M1]"]);
    expect(boardOf(cli).cards).toEqual([
      { id: 1, title: "Engine", description: "The core", column: "doing", milestone: "M1", createdAt: DAY, updatedAt: DAY, steps: [] },
    ]);
    expect(cli.file.endsWith("}\n")).toBe(true);
  });

  it("defaults to later and prints no empty milestone", () => {
    const cli = fake();
    expect(run(["add", "Editor"], cli.io)).toBe(0);
    expect(cli.out).toEqual(["added: #1  later  Editor"]);
  });

  it("rejects zero or several titles without writing", () => {
    for (const args of [["add"], ["add", "a", "b"]]) {
      const cli = fake();
      expect(run(args, cli.io)).toBe(2);
      expect(cli.err).toEqual(["error: expected exactly one argument: title (see: roadmap help)"]);
      expect(cli.writes).toBe(0);
    }
  });

  it("rejects an unknown column", () => {
    const cli = fake();
    expect(run(["add", "T", "--column", "todo"], cli.io)).toBe(2);
    expect(cli.err).toEqual(["error: unknown column: todo (expected: later, next, doing, done) (see: roadmap help)"]);
  });
});

describe("list", () => {
  it("lists all cards or those of one column", () => {
    const cli = seeded();
    expect(run(["list"], cli.io)).toBe(0);
    expect(cli.out).toEqual(["#1  doing  Engine [M1]", "#2  later  Editor"]);
    cli.out.length = 0;
    expect(run(["list", "--column", "later"], cli.io)).toBe(0);
    expect(cli.out).toEqual(["#2  later  Editor"]);
    expect(cli.writes).toBe(0);
  });

  it("reports an empty list", () => {
    const cli = seeded();
    expect(run(["list", "--column", "done"], cli.io)).toBe(0);
    expect(cli.out).toEqual(["(no cards)"]);
  });

  it("rejects an unknown column", () => {
    const cli = seeded();
    expect(run(["list", "--column", "x"], cli.io)).toBe(2);
    expect(cli.err).toEqual(["error: unknown column: x (expected: later, next, doing, done) (see: roadmap help)"]);
  });
});

describe("show", () => {
  it("shows a card with its description and dates", () => {
    const cli = seeded();
    expect(run(["show", "#1"], cli.io)).toBe(0);
    expect(cli.out).toEqual(["#1  doing  Engine [M1]", "The core", `created ${DAY}, updated ${DAY}`]);
  });

  it("prints no empty description", () => {
    const cli = seeded();
    expect(run(["show", "#2"], cli.io)).toBe(0);
    expect(cli.out).toEqual(["#2  later  Editor", `created ${DAY}, updated ${DAY}`]);
  });

  it("rejects an invalid id", () => {
    for (const command of ["show", "edit", "rm"]) {
      const cli = seeded();
      expect(run([command, "abc", "--title", "x"].slice(0, command === "edit" ? 4 : 2), cli.io)).toBe(2);
      expect(cli.err).toEqual(["error: invalid id: abc (see: roadmap help)"]);
      expect(cli.writes).toBe(0);
    }
    const moved = seeded();
    expect(run(["move", "R-1", "done"], moved.io)).toBe(2);
    expect(moved.err).toEqual(["error: invalid id: R-1 (see: roadmap help)"]);
  });

  it("rejects an unknown card or a missing id", () => {
    const cli = seeded();
    expect(run(["show", "#9"], cli.io)).toBe(1);
    expect(run(["show"], cli.io)).toBe(2);
    expect(cli.err).toEqual(["error: card not found: #9", "error: expected exactly one argument: id (see: roadmap help)"]);
  });
});

describe("move", () => {
  it("moves a card to a position", () => {
    const cli = seeded();
    expect(run(["move", "2", "doing", "--position", "0"], cli.io)).toBe(0);
    expect(cli.out).toEqual(["moved: #2 → doing"]);
    expect(boardOf(cli).cards.map((card) => `${String(card.id)}:${card.column}`)).toEqual(["2:doing", "1:doing"]);
    expect(cli.writes).toBe(1);
  });

  it("moves to the end of the column without a position", () => {
    const cli = seeded();
    expect(run(["move", "1", "done"], cli.io)).toBe(0);
    expect(boardOf(cli).cards.map((card) => `${String(card.id)}:${card.column}`)).toEqual(["2:later", "1:done"]);
  });

  it("rejects missing or extra arguments", () => {
    for (const args of [["move"], ["move", "1"], ["move", "1", "done", "x"]]) {
      const cli = seeded();
      expect(run(args, cli.io)).toBe(2);
      expect(cli.err).toEqual(["error: expected: move <id> <column> (see: roadmap help)"]);
      expect(cli.writes).toBe(0);
    }
  });

  it("rejects an unknown column", () => {
    const cli = seeded();
    expect(run(["move", "1", "x"], cli.io)).toBe(2);
    expect(cli.err).toEqual(["error: unknown column: x (expected: later, next, doing, done) (see: roadmap help)"]);
  });

  it.each(["-1", "1a", "a1", "", "1.5", " 1"])("rejects a non-numeric position: %j", (position) => {
    const cli = seeded();
    expect(run(["move", "1", "done", `--position=${position}`], cli.io)).toBe(2);
    expect(cli.err).toEqual([`error: invalid position: ${position} (see: roadmap help)`]);
  });

  it("accepts multi-digit positions, then bounds them", () => {
    const cli = seeded();
    expect(run(["move", "1", "done", "--position", "10"], cli.io)).toBe(1);
    expect(cli.err).toEqual(["error: invalid position: 10 (0 to 0)"]);
  });
});

describe("edit", () => {
  it("edits the given fields", () => {
    const cli = seeded();
    expect(run(["edit", "#2", "--title", "Web editor", "--desc", "Patching", "--milestone", "M2"], cli.io)).toBe(0);
    expect(cli.out).toEqual(["edited: #2"]);
    expect(boardOf(cli).cards[1]).toMatchObject({ title: "Web editor", description: "Patching", milestone: "M2" });
  });

  it("rejects an empty edit", () => {
    const cli = seeded();
    expect(run(["edit", "#2"], cli.io)).toBe(1);
    expect(cli.err).toEqual(["error: nothing to edit"]);
    expect(cli.writes).toBe(0);
  });

  it("rejects a missing id", () => {
    const cli = seeded();
    expect(run(["edit", "--title", "x"], cli.io)).toBe(2);
    expect(cli.err).toEqual(["error: expected exactly one argument: id (see: roadmap help)"]);
  });
});

describe("rm", () => {
  it("removes a card", () => {
    const cli = seeded();
    expect(run(["rm", "1"], cli.io)).toBe(0);
    expect(cli.out).toEqual(["removed: #1"]);
    expect(boardOf(cli).cards.map((card) => card.id)).toEqual([2]);
  });

  it("rejects an unknown card", () => {
    const cli = seeded();
    expect(run(["rm", "9"], cli.io)).toBe(1);
    expect(cli.err).toEqual(["error: card not found: #9"]);
  });

  it("rejects a missing id", () => {
    const cli = seeded();
    expect(run(["rm"], cli.io)).toBe(2);
    expect(cli.err).toEqual(["error: expected exactly one argument: id (see: roadmap help)"]);
  });
});

describe("title", () => {
  it("sets the board title, trimmed, and confirms", () => {
    const cli = seeded();
    expect(run(["title", "  Poyetik "], cli.io)).toBe(0);
    expect(cli.out).toEqual(["title: Poyetik"]);
    expect(boardOf(cli)).toMatchObject({ title: "Poyetik", nextId: 3 });
    expect(boardOf(cli).cards.map((card) => card.id)).toEqual([1, 2]);
    expect(cli.writes).toBe(1);
  });

  it("rejects an empty title without writing", () => {
    const cli = seeded();
    expect(run(["title", "  "], cli.io)).toBe(1);
    expect(cli.err).toEqual(["error: title must be a non-empty string"]);
    expect(cli.writes).toBe(0);
  });

  it("rejects zero or several titles", () => {
    for (const args of [["title"], ["title", "a", "b"]]) {
      const cli = seeded();
      expect(run(args, cli.io)).toBe(2);
      expect(cli.err).toEqual(["error: expected exactly one argument: title (see: roadmap help)"]);
      expect(cli.writes).toBe(0);
    }
  });
});

describe("validate", () => {
  it("confirms a valid board", () => {
    const cli = seeded();
    expect(run(["validate"], cli.io)).toBe(0);
    expect(cli.out).toEqual(["valid board: 2 card(s)"]);
    expect(cli.writes).toBe(0);
  });

  it("reports unreadable JSON", () => {
    const cli = fake("{oops");
    expect(run(["validate"], cli.io)).toBe(1);
    expect(cli.err).toHaveLength(1);
    expect(cli.err[0]).toMatch(/^error: unreadable board\.json: .+/);
  });

  it("includes the JSON error in the message", () => {
    const cli = fake("");
    expect(run(["validate"], cli.io)).toBe(1);
    expect(cli.err).toEqual(["error: unreadable board.json: SyntaxError: Unexpected end of JSON input"]);
  });

  it("lets disk read failures propagate", () => {
    const cli = fake();
    const io: Io = {
      ...cli.io,
      read: () => {
        throw new Error("disk missing");
      },
    };
    expect(() => run(["validate"], io)).toThrow("disk missing");
    expect(cli.err).toEqual([]);
  });

  it("reports an invalid board, naming its file", () => {
    const cli = fake('{"version": 2}');
    expect(run(["validate"], cli.io)).toBe(1);
    expect(cli.err).toEqual(["error: invalid board.json: board: expected fields cards, nextId, title, version, got version"]);
  });
});

describe("general errors", () => {
  it.each(["frobnicate", "toString", "constructor"])("rejects an unknown command: %s", (command) => {
    const cli = fake();
    expect(run([command], cli.io)).toBe(2);
    expect(cli.err).toEqual([`error: unknown command: ${command} (see: roadmap help)`]);
  });

  it("has no task command: steps are managed with step", () => {
    const cli = seeded();
    expect(run(["task", "add", "1", "Install"], cli.io)).toBe(2);
    expect(cli.err).toEqual(["error: unknown command: task (see: roadmap help)"]);
    expect(cli.writes).toBe(0);
    expect(HELP).not.toMatch(/\btask\b/);
  });

  it("rejects an unknown option", () => {
    const cli = fake();
    expect(run(["list", "--nope"], cli.io)).toBe(2);
    expect(cli.err).toHaveLength(1);
    expect(cli.err[0]).toMatch(/^error: .*--nope/);
  });

  it("rejects an option given without its value", () => {
    const cli = seeded();
    expect(run(["add", "T", "--desc"], cli.io)).toBe(2);
    expect(cli.err).toHaveLength(1);
    expect(cli.err[0]).toMatch(/^error: .*--desc.* \(see: roadmap help\)$/);
    expect(cli.writes).toBe(0);
  });

  it.each([
    [["help", "list"], "help"],
    [["list", "doing"], "list"],
    [["validate", "x"], "validate"],
  ])("rejects an argument to a command that takes none: %j", (args, command) => {
    const cli = seeded();
    expect(run(args, cli.io)).toBe(2);
    expect(cli.err).toEqual([`error: ${command} takes no argument, got: ${String(args[1])} (see: roadmap help)`]);
    expect(cli.out).toEqual([]);
  });

  it.each([
    [["help", "--column", "x"], "--column", "help"],
    [["list", "--title", "x"], "--title", "list"],
    [["show", "1", "--desc", "x"], "--desc", "show"],
    [["add", "T", "--position", "0"], "--position", "add"],
    [["move", "1", "done", "--column", "next"], "--column", "move"],
    [["edit", "1", "--column", "done"], "--column", "edit"],
    [["rm", "1", "--title", "x"], "--title", "rm"],
    [["title", "T", "--milestone", "x"], "--milestone", "title"],
    [["step", "add", "1", "S", "--title", "x"], "--title", "step"],
    [["validate", "--column", "done"], "--column", "validate"],
  ])("rejects an option that does not apply, without writing: %j", (args, option, command) => {
    const cli = seeded();
    expect(run(args, cli.io)).toBe(2);
    expect(cli.err).toEqual([`error: option ${option} does not apply to ${command} (see: roadmap help)`]);
    expect(cli.out).toEqual([]);
    expect(cli.writes).toBe(0);
  });

  it("accepts every option of each command", () => {
    const cli = seeded();
    expect(run(["list", "--column", "doing"], cli.io)).toBe(0);
    expect(run(["add", "T", "--desc", "d", "--column", "next", "--milestone", "M"], cli.io)).toBe(0);
    expect(run(["move", "3", "done", "--position", "0"], cli.io)).toBe(0);
    expect(run(["edit", "3", "--title", "U", "--desc", "e", "--milestone", "N"], cli.io)).toBe(0);
    expect(cli.err).toEqual([]);
  });

  it("takes a value starting with a dash after --, as the help says", () => {
    const cli = seeded();
    expect(run(["step", "add", "1", "--", "-leading"], cli.io)).toBe(0);
    expect(boardOf(cli).cards[0]?.steps).toEqual([{ title: "-leading", done: false }]);
    expect(HELP).toContain("roadmap step add 3 -- -title");
  });

  it("rethrows unexpected errors", () => {
    const cli = fake();
    const io: Io = {
      ...cli.io,
      today: () => {
        throw new RangeError("broken clock");
      },
    };
    expect(() => run(["add", "T"], io)).toThrow(RangeError);
  });

  it("rethrows a TypeError without a code", () => {
    const cli = fake();
    const io: Io = {
      ...cli.io,
      today: () => {
        throw new TypeError("bug");
      },
    };
    expect(() => run(["add", "T"], io)).toThrow("bug");
  });
});

describe("step", () => {
  /** Builds the seeded fake with three steps on card #1 (Install, Build, Ship), output and write count reset. */
  function withSteps(): Fake {
    const cli = seeded();
    expect(run(["step", "add", "1", "Install"], cli.io)).toBe(0);
    expect(run(["step", "add", "#1", "Build"], cli.io)).toBe(0);
    expect(run(["step", "add", "1", "Ship"], cli.io)).toBe(0);
    cli.out.length = 0;
    cli.writes = 0;
    return cli;
  }

  it("adds steps and prints the card with statuses", () => {
    const cli = seeded();
    expect(run(["step", "add", "1", "Install"], cli.io)).toBe(0);
    expect(cli.out).toEqual(["#1  doing  Engine [M1]  0/1", "The core", "  1. Install  [In progress]"]);
    expect(boardOf(cli).cards[0]?.steps).toEqual([{ title: "Install", done: false }]);
  });

  it("shows no step in progress for a card outside the doing column", () => {
    const cli = seeded();
    expect(run(["step", "add", "2", "Sketch"], cli.io)).toBe(0);
    expect(cli.out).toEqual(["#2  later  Editor  0/1", "  1. Sketch"]);
    expect(run(["step", "done", "2"], cli.io)).toBe(1);
    expect(cli.err).toEqual(["error: #2 has no step in progress"]);
  });

  it("completes the step in progress and moves the tag to the next one", () => {
    const cli = withSteps();
    expect(run(["step", "done", "1"], cli.io)).toBe(0);
    expect(cli.out).toEqual([
      "#1  doing  Engine [M1]  1/3",
      "The core",
      "  1. Install  [Done]",
      "  2. Build  [In progress]",
      "  3. Ship",
    ]);
    expect(cli.writes).toBe(1);
  });

  it("completes, reopens, renames and removes a given step", () => {
    const cli = withSteps();
    expect(run(["step", "done", "1", "3"], cli.io)).toBe(0);
    expect(boardOf(cli).cards[0]?.steps.map((step) => step.done)).toEqual([false, false, true]);
    expect(run(["step", "undo", "1", "3"], cli.io)).toBe(0);
    expect(boardOf(cli).cards[0]?.steps.map((step) => step.done)).toEqual([false, false, false]);
    expect(run(["step", "edit", "1", "2", "Compile"], cli.io)).toBe(0);
    expect(run(["step", "rm", "1", "1"], cli.io)).toBe(0);
    expect(boardOf(cli).cards[0]?.steps).toEqual([
      { title: "Compile", done: false },
      { title: "Ship", done: false },
    ]);
    expect(cli.out.at(-1)).toBe("  2. Ship");
  });

  it("shows steps in show and progress in list", () => {
    const cli = withSteps();
    expect(run(["step", "done", "1"], cli.io)).toBe(0);
    cli.out.length = 0;
    expect(run(["list"], cli.io)).toBe(0);
    expect(cli.out).toEqual(["#1  doing  Engine [M1]  1/3", "#2  later  Editor"]);
    cli.out.length = 0;
    expect(run(["show", "1"], cli.io)).toBe(0);
    expect(cli.out).toEqual([
      "#1  doing  Engine [M1]  1/3",
      "The core",
      "  1. Install  [Done]",
      "  2. Build  [In progress]",
      "  3. Ship",
      `created ${DAY}, updated ${DAY}`,
    ]);
  });

  it.each([
    [["step"], "unknown step action: (none) (expected: add, done, undo, edit, rm)"],
    [["step", "close", "1"], "unknown step action: close (expected: add, done, undo, edit, rm)"],
    [["step", "toString", "1"], "unknown step action: toString (expected: add, done, undo, edit, rm)"],
  ])("rejects an unknown action: %j", (args, message) => {
    const cli = withSteps();
    expect(run(args, cli.io)).toBe(2);
    expect(cli.err).toEqual([`error: ${message} (see: roadmap help)`]);
    expect(cli.writes).toBe(0);
  });

  it.each([
    [["step", "add"], "step add <id> <title>"],
    [["step", "add", "1"], "step add <id> <title>"],
    [["step", "add", "1", "a", "b"], "step add <id> <title>"],
    [["step", "done"], "step done <id> [n]"],
    [["step", "done", "1", "2", "3"], "step done <id> [n]"],
    [["step", "undo", "1"], "step undo <id> <n>"],
    [["step", "undo", "1", "2", "3"], "step undo <id> <n>"],
    [["step", "edit", "1", "2"], "step edit <id> <n> <title>"],
    [["step", "edit", "1", "2", "a", "b"], "step edit <id> <n> <title>"],
    [["step", "rm", "1"], "step rm <id> <n>"],
    [["step", "rm", "1", "2", "3"], "step rm <id> <n>"],
  ])("rejects wrong arguments: %j", (args, usage) => {
    const cli = withSteps();
    expect(run(args, cli.io)).toBe(2);
    expect(cli.err).toEqual([`error: expected: ${usage} (see: roadmap help)`]);
    expect(cli.writes).toBe(0);
  });

  it("rejects invalid ids and step numbers", () => {
    const cli = withSteps();
    expect(run(["step", "add", "x", "T"], cli.io)).toBe(2);
    expect(run(["step", "done", "1", "x"], cli.io)).toBe(2);
    expect(run(["step", "undo", "x", "1"], cli.io)).toBe(2);
    expect(run(["step", "edit", "1", "0", "T"], cli.io)).toBe(2);
    expect(run(["step", "rm", "1", "9"], cli.io)).toBe(1);
    expect(cli.err).toEqual([
      "error: invalid id: x (see: roadmap help)",
      "error: invalid step number: x (see: roadmap help)",
      "error: invalid id: x (see: roadmap help)",
      "error: invalid step number: 0 (see: roadmap help)",
      "error: invalid step number for #1: 9 (1 to 3)",
    ]);
    expect(cli.writes).toBe(0);
  });
});

describe("guardStreams", () => {
  /** A stream that records its error listeners and what is written to it. */
  function stream(): ErrorSource & { write: (text: string) => void; emit: (error: Error) => void; written: string[] } {
    const listeners: ((error: Error) => void)[] = [];
    const written: string[] = [];
    return {
      on: (_event, listener) => listeners.push(listener),
      write: (text) => {
        written.push(text);
      },
      emit: (error) => {
        listeners.forEach((listener) => {
          listener(error);
        });
      },
      written,
    };
  }

  /** Two guarded fake streams and a count of the failures reported. */
  function guarded(): { stdout: ReturnType<typeof stream>; stderr: ReturnType<typeof stream>; failures: () => number } {
    const stdout = stream();
    const stderr = stream();
    let failures = 0;
    guardStreams(stdout, stderr, () => {
      failures += 1;
    });
    return { stdout, stderr, failures: () => failures };
  }

  /** A stream error as Node reports it, with its system code. */
  const failed = (code: string): Error => Object.assign(new Error(`write ${code}`), { code });

  it("drops a closed stdout silently", () => {
    const { stdout, stderr, failures } = guarded();
    stdout.emit(failed("EPIPE"));
    expect(failures()).toBe(0);
    expect(stderr.written).toStrictEqual([]);
  });

  it("reports any other stdout failure as one error line, then as a failure", () => {
    const { stdout, stderr, failures } = guarded();
    stdout.emit(failed("EIO"));
    expect(stderr.written).toStrictEqual(["error: cannot write to stdout: EIO\n"]);
    expect(failures()).toBe(1);
    stdout.emit(new Error("broken"));
    expect(stderr.written.at(-1)).toBe("error: cannot write to stdout: broken\n");
    expect(failures()).toBe(2);
  });

  it("ignores every stderr failure", () => {
    const { stdout, stderr, failures } = guarded();
    expect(() => {
      stderr.emit(failed("EPIPE"));
      stderr.emit(failed("EIO"));
    }).not.toThrow();
    expect(failures()).toBe(0);
    expect(stdout.written).toStrictEqual([]);
    expect(stderr.written).toStrictEqual([]);
  });
});

describe("exitAfterStreamFailure", () => {
  it("turns a success into exit code 1 and never replaces a failure's code", () => {
    expect(exitAfterStreamFailure(0)).toBe(1);
    expect(exitAfterStreamFailure(1)).toBe(1);
    expect(exitAfterStreamFailure(2)).toBe(2);
  });
});

describe("failure", () => {
  it("names a system error by its stable code, without repeating the path", () => {
    const missing = Object.assign(new Error("ENOENT: no such file or directory, open '/x'"), { code: "ENOENT" });
    expect(failure(missing)).toBe("ENOENT");
  });

  it("falls back to the message, then to the text of anything thrown", () => {
    expect(failure(new Error("disk missing"))).toBe("disk missing");
    expect(failure(Object.assign(new Error("odd"), { code: 5 }))).toBe("odd");
    expect(failure("plain")).toBe("plain");
  });
});

describe("UsageError", () => {
  it("is a named Error", () => {
    expect(new UsageError("x")).toMatchObject({ name: "UsageError", message: "x" });
  });
});
