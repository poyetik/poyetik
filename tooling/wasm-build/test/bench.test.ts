import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { devNull, tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, test } from "vitest";
import { benchMain, type BenchIo, type Benchmarks, call, counterValue, countedInstance, exact, forBenchmark, formatBaseline, isCount, type Measurement, parseBaseline, regressions } from "../src/bench.ts";
import { instrument } from "../src/instrument.ts";
import { assemble, I32_GLOBAL } from "../src/testing/assemble.ts";

/** Two measurements, in their order. */
const MEASURED: readonly Measurement[] = [
  { name: "render 64x36", value: 1000, unit: "instructions" },
  { name: "module", value: 500, unit: "bytes" },
];

/** The baseline of {@link MEASURED}, as bless writes it. */
const BLESSED = '{\n  "render 64x36": 1000,\n  "module": 500\n}\n';

/**
 * Where a hint runs, from the shared test matrix (`conformance/test-matrix.json`, found by walking
 * up, since mutation testing runs the tests from a copy nested deeper): every directory with the
 * damage staged or not, in a repository of that name.
 */
const HINT_RUNS = ((): { readonly repository: string; readonly runs: readonly { readonly directory: string; readonly staged: boolean }[] } => {
  for (let folder = new URL("./", import.meta.url); folder.pathname !== "/"; folder = new URL("../", folder)) {
    const file = new URL("conformance/test-matrix.json", folder);
    if (existsSync(file)) {
      const matrix: unknown = JSON.parse(readFileSync(file, "utf8"));
      const value = (key: string): unknown => Reflect.get(Object(matrix), key);
      const [repository, directories, staged] = [value("hint_repository"), value("hint_directories"), value("hint_staged")];
      if (typeof repository !== "string" || !Array.isArray(directories) || !directories.every((item) => typeof item === "string") || !Array.isArray(staged) || !staged.every((item) => typeof item === "boolean")) {
        throw new Error("invalid test matrix: where a hint runs");
      }
      return { repository, runs: directories.flatMap((directory) => staged.map((state) => ({ directory, staged: state }))) };
    }
  }
  throw new Error("conformance/test-matrix.json not found");
})();

/** A fake outside world: the files by path, what was written, what was printed. */
function fakeIo(stored: Readonly<Record<string, Uint8Array>> = {}, writable = true): { io: BenchIo; out: string[]; err: string[]; written: Map<string, string> } {
  const out: string[] = [];
  const err: string[] = [];
  const written = new Map<string, string>();
  const io: BenchIo = {
    repository: "/repo",
    read: (path) => {
      const file = stored[path];
      if (file === undefined) {
        throw Object.assign(new Error(`ENOENT ${path}`), { code: "ENOENT" });
      }
      return file;
    },
    write: (path, text) => {
      if (!writable) {
        throw new Error("EACCES");
      }
      written.set(path, text);
    },
    out: (line) => out.push(line),
    err: (line) => err.push(line),
  };
  return { io, out, err, written };
}

/** Benchmarks measuring `measured` (or throwing it), obeying laws that report `broken`. */
function benchmarks(measured: readonly Measurement[] | Error = MEASURED, broken: readonly string[] = []): Benchmarks {
  return {
    baselineFile: "spec/b.json",
    moduleFile: "dist/m.wasm",
    packageName: "@poyetik/m",
    tolerancePercent: 2,
    measure: (wasm) => {
      expect(Array.from(wasm)).toStrictEqual([0, 97, 115, 109]);
      if (measured instanceof Error) {
        throw measured;
      }
      return measured;
    },
    laws: () => broken,
  };
}

/** The module and baseline files of a repository at `/repo`. */
function files(baseline: string = BLESSED): Record<string, Uint8Array> {
  return { "/repo/dist/m.wasm": Uint8Array.of(0, 97, 115, 109), "/repo/spec/b.json": new TextEncoder().encode(baseline) };
}

/**
 * The variables under which a test runs git without the machine's or the user's
 * configuration: a copy of xtask's `HERMETIC_GIT` (`crates/xtask/src/runner.rs`), which its
 * hermetic git gate holds this file to. `GIT_CONFIG_GLOBAL` needs git 2.32 or later.
 */
const HERMETIC_GIT: Readonly<Record<string, string>> = {
  "GIT_CONFIG_NOSYSTEM": "1",
  "GIT_CONFIG_GLOBAL": devNull,
  "GIT_ATTR_NOSYSTEM": "1",
  "XDG_CONFIG_HOME": devNull,
};

/**
 * The variables git sets for a hook that name one repository, never passed on, so that the
 * process running the tests leaks none in: a copy of xtask's `LOCAL_GIT_VARIABLES`, which its
 * hermetic git gate holds this file to.
 */
const LOCAL_GIT_VARIABLES: readonly string[] = [
  "GIT_ALTERNATE_OBJECT_DIRECTORIES",
  "GIT_CONFIG",
  "GIT_CONFIG_PARAMETERS",
  "GIT_CONFIG_COUNT",
  "GIT_OBJECT_DIRECTORY",
  "GIT_DIR",
  "GIT_WORK_TREE",
  "GIT_IMPLICIT_WORK_TREE",
  "GIT_GRAFT_FILE",
  "GIT_INDEX_FILE",
  "GIT_REPLACE_REF_BASE",
  "GIT_PREFIX",
  "GIT_SHALLOW_FILE",
  "GIT_COMMON_DIR",
];

/** The environment of a git a test starts: this process's, without git's local variables, under {@link HERMETIC_GIT}. */
function hermeticGit(): NodeJS.ProcessEnv {
  const kept = Object.entries(process.env).filter(([name]) => !LOCAL_GIT_VARIABLES.includes(name));
  return { ...Object.fromEntries(kept), ...HERMETIC_GIT };
}

describe("benchMain", () => {
  test("refuses no command, another command and an argument, as usage errors", () => {
    for (const [args, line] of [
      [[], "error: no command given (see: pnpm --filter @poyetik/m exec node scripts/benchmarks.ts help)"],
      [["run"], 'error: unknown command "run" (see: pnpm --filter @poyetik/m exec node scripts/benchmarks.ts help)'],
      [["check", "now"], "error: `check` takes no argument (see: pnpm --filter @poyetik/m exec node scripts/benchmarks.ts help)"],
    ] as const) {
      const { io, out, err } = fakeIo();
      expect(benchMain(io, args, benchmarks()), line).toBe(2);
      expect([out, err], line).toStrictEqual([[], [line]]);
    }
  });

  test("prints its usage for help", () => {
    const { io, out, err } = fakeIo();
    expect(benchMain(io, ["help"], benchmarks())).toBe(0);
    expect(out).toStrictEqual([
      "usage: pnpm --filter @poyetik/m exec node scripts/benchmarks.ts <check|bless|help>\n  check  measure dist/m.wasm and compare it with spec/b.json, within ±2%, and with its laws\n  bless  measure it and rewrite spec/b.json, if its laws hold, only through cargo xtask bless-benchmarks\n  help   print this help",
    ]);
    expect(err).toStrictEqual([]);
  });

  test("checks the measurements against the baseline and the laws", () => {
    const agreeing = fakeIo(files());
    expect(benchMain(agreeing.io, ["check"], benchmarks())).toBe(0);
    expect([agreeing.out, agreeing.err]).toStrictEqual([["render 64x36: 1000 instructions", "module: 500 bytes"], []]);
    const lawless = fakeIo(files());
    expect(benchMain(lawless.io, ["check"], benchmarks(MEASURED, ["render is not linear"]))).toBe(1);
    expect(lawless.out.at(-1)).toBe("render is not linear");
    expect(lawless.err).toStrictEqual(["error: the benchmarks of dist/m.wasm moved beyond ±2% of spec/b.json, or break their laws (1 problem(s); see spec/benchmarks.md)"]);
    const moved = fakeIo(files('{\n  "render 64x36": 900,\n  "gone": 1\n}\n'));
    expect(benchMain(moved.io, ["check"], benchmarks())).toBe(1);
    expect(moved.out.slice(2)).toStrictEqual([
      "render 64x36: 1000 instructions, baseline 900 (+11.12%, beyond ±2%)",
      "module: 500 bytes, not in the baseline",
      "gone: in the baseline, measured by no benchmark",
    ]);
    // Finding K7-1 of the sixth deep review of visual block 2 (#34): the line named no command. The
    // bless is named only where it would write: it refuses broken laws.
    expect(moved.err).toStrictEqual(["error: the benchmarks of dist/m.wasm moved beyond ±2% of spec/b.json: bless them only when the change is meant (see: cargo xtask bless-benchmarks) (3 problem(s); see spec/benchmarks.md)"]);
  });

  test("blesses the measurements when their laws hold, and only then", () => {
    const blessed = fakeIo(files());
    expect(benchMain(blessed.io, ["bless"], benchmarks())).toBe(0);
    expect(blessed.written).toStrictEqual(new Map([["/repo/spec/b.json", BLESSED]]));
    expect(blessed.out).toStrictEqual(["render 64x36: 1000 instructions", "module: 500 bytes", "wrote spec/b.json"]);
    const refused = fakeIo(files());
    expect(benchMain(refused.io, ["bless"], benchmarks(MEASURED, ["a law", "another"]))).toBe(1);
    expect(refused.written.size).toBe(0);
    expect(refused.err).toStrictEqual(["error: no spec/b.json written: the benchmarks of dist/m.wasm break their laws (2 problem(s); see spec/benchmarks.md)"]);
    const readOnly = fakeIo(files(), false);
    expect(benchMain(readOnly.io, ["bless"], benchmarks())).toBe(1);
    expect(readOnly.err).toStrictEqual(["error: cannot write spec/b.json: EACCES"]);
  });

  test("says what helps a baseline it cannot read, by the code of the error", () => {
    // Findings H13-3 and J1-n2 of the fifth deep review of visual block 2 (#34): a missing or
    // unreadable baseline was named with no remedy.
    for (const [code, remedy] of [
      ["EISDIR", ": a directory stands where the file should be: move it away, then check again (see: pnpm --filter @poyetik/m benchmarks:check)"],
      ["EACCES", ": make it readable, then check again (see: pnpm --filter @poyetik/m benchmarks:check)"],
      ["EPERM", ": make it readable, then check again (see: pnpm --filter @poyetik/m benchmarks:check)"],
      ["EIO", ""],
    ] as const) {
      const fake = fakeIo();
      const refusing: BenchIo = {
        ...fake.io,
        read: () => {
          throw Object.assign(new Error(code), { code });
        },
      };
      expect(benchMain(refusing, ["check"], benchmarks()), code).toBe(1);
      expect(fake.err, code).toStrictEqual([`error: cannot read spec/b.json: ${code}${remedy}`]);
    }
    const { io, err } = fakeIo();
    const thrown: BenchIo = {
      ...io,
      read: () => {
        throw new Error("no code");
      },
    };
    expect(benchMain(thrown, ["check"], benchmarks())).toBe(1);
    expect(err).toStrictEqual(["error: cannot read spec/b.json: no code"]);
  });

  test("cuts no remedy by a long reason, drops no byte order mark, and builds only a missing module", () => {
    // Finding K13-2 of the sixth deep review of visual block 2 (#34): the line was bounded whole,
    // so a damaged file's long name hid the command that helps.
    const long = fakeIo(files(`{"${"x".repeat(2000)}": 0}`));
    expect(benchMain(long.io, ["check"], benchmarks())).toBe(1);
    expect(long.err).toStrictEqual([`error: spec/b.json: ${"x".repeat(986)}…: restore it (see: git checkout HEAD -- :/spec/b.json), or rewrite it only when the counts are meant to change (see: cargo xtask bless-benchmarks)`]);
    // K2-2: a byte order mark was dropped, and the file read as bless writes it.
    const marked = fakeIo(files(`\u{feff}${BLESSED}`));
    expect(benchMain(marked.io, ["check"], benchmarks())).toBe(1);
    expect(marked.err).toStrictEqual([expect.stringMatching(/^error: spec\/b\.json: not JSON \(/u)]);
    // K13-9: a module that cannot be read for another reason was told to be built.
    const { io, err } = fakeIo(files());
    const refusing: BenchIo = {
      ...io,
      read: (path) => {
        if (path.endsWith(".wasm")) {
          throw Object.assign(new Error("EACCES"), { code: "EACCES" });
        }
        return io.read(path);
      },
    };
    expect(benchMain(refusing, ["check"], benchmarks())).toBe(1);
    expect(err).toStrictEqual(["error: cannot read dist/m.wasm: EACCES"]);
  });

  test("names the file it cannot read, the module it cannot measure, and a baseline not as bless writes it", () => {
    for (const [given, measured, args, line] of [
      [{}, MEASURED, ["check"], "error: cannot read spec/b.json: ENOENT /repo/spec/b.json: restore it (see: git checkout HEAD -- :/spec/b.json), or bless it (see: cargo xtask bless-benchmarks)"],
      [{ "/repo/spec/b.json": new TextEncoder().encode(BLESSED) }, MEASURED, ["check"], "error: cannot read dist/m.wasm: ENOENT /repo/dist/m.wasm: build it (see: pnpm --filter @poyetik/m build:wasm)"],
      [{ ...files(), "/repo/spec/b.json": Uint8Array.of(0xff) }, MEASURED, ["check"], "error: spec/b.json: not UTF-8 text, not as bless writes it (The encoded data was not valid for encoding utf-8): restore it (see: git checkout HEAD -- :/spec/b.json), or rewrite it only when the counts are meant to change (see: cargo xtask bless-benchmarks)"],
      [files('{"module": 500}'), MEASURED, ["check"], "error: spec/b.json: not as bless writes it (a name written twice, or another layout): restore it (see: git checkout HEAD -- :/spec/b.json), or rewrite it only when the counts are meant to change (see: cargo xtask bless-benchmarks)"],
      [files("[]"), MEASURED, ["check"], "error: spec/b.json: not a JSON object of counts by benchmark name: restore it (see: git checkout HEAD -- :/spec/b.json), or rewrite it only when the counts are meant to change (see: cargo xtask bless-benchmarks)"],
      [files(), new Error("render 64x36: refused"), ["bless"], "error: dist/m.wasm: render 64x36: refused"],
    ] as const) {
      const { io, err } = fakeIo(given);
      expect(benchMain(io, args, benchmarks(measured)), line).toBe(1);
      expect(err, line).toStrictEqual([line]);
    }
  });
});

/** A repository of one test: its root, its command, and how a hint is followed in it. */
interface Repository {
  /** Where it stands. */
  readonly root: string;
  /** Runs the command, giving its exit code, the `error:` line it wrote if any, and every hint of it. */
  readonly bench: (command: string) => { readonly status: number; readonly line: string | undefined; readonly hints: readonly string[] };
  /** Runs `command` through a POSIX shell in `directory`, the command's own two being run on this repository. */
  readonly follow: (command: string, directory: string) => void;
}

/**
 * Runs `body` in a repository whose path holds a space and a quote, holding the committed
 * baseline and the module, removed afterwards. Git reads no configuration of the machine's or
 * the user's ({@link hermeticGit}).
 */
function withRepository(body: (repository: Repository) => void): void {
  const root = mkdtempSync(join(tmpdir(), `bench-${HINT_RUNS.repository}-`));
  /** Runs `command` through a POSIX shell in `cwd`, a home of its own. */
  const shell = (cwd: string, command: string): void => {
    execFileSync("sh", ["-c", command], { cwd, env: { ...hermeticGit(), HOME: root }, stdio: "pipe" });
  };
  /** Runs the command on the files of `root`. */
  const bench: Repository["bench"] = (command) => {
    const err: string[] = [];
    const io: BenchIo = { repository: root, read: (path) => new Uint8Array(readFileSync(path)), write: writeFileSync, out: () => undefined, err: (line) => err.push(line) };
    const status = benchMain(io, [command], benchmarks());
    return { status, line: err[0], hints: [...(err[0] ?? "").matchAll(/\(see: (?<command>[^)]+)\)/gu)].map((hint) => hint.groups?.["command"] ?? "") };
  };
  try {
    for (const folder of ["spec", "dist"]) {
      mkdirSync(join(root, folder));
    }
    writeFileSync(join(root, "dist", "m.wasm"), Uint8Array.of(0, 97, 115, 109));
    writeFileSync(join(root, "spec", "b.json"), BLESSED);
    shell(root, "git -c user.name=bench -c user.email=bench@example.invalid init -q && git add . && git -c user.name=bench -c user.email=bench@example.invalid commit -q -m baseline");
    body({
      root,
      bench,
      follow: (command, directory) => {
        if (command === "cargo xtask bless-benchmarks") {
          expect(bench("bless")).toStrictEqual({ status: 0, line: undefined, hints: [] });
        } else if (command === "pnpm --filter @poyetik/m benchmarks:check") {
          bench("check");
        } else {
          mkdirSync(join(root, directory), { recursive: true });
          shell(join(root, directory), command);
        }
      },
    });
  } finally {
    rmSync(root, { recursive: true });
  }
}

describe("the hints a check names", () => {
  test("holds a place from the root and from below, staged and not", () => {
    expect(new Set(HINT_RUNS.runs.map(({ directory, staged }) => `${String(directory === "")} ${String(staged)}`)).size).toBe(4);
  });

  test.each(HINT_RUNS.runs)("puts the baseline right, followed as written from '$directory', staged: $staged", ({ directory, staged }) => {
    // Findings H1-1 and H14-3 of the fifth deep review of visual block 2 (#34): the hint
    // restored from the index, which keeps a damaged file once staged. L1-n1 and K5-5 of the
    // sixth: only the first hint was followed, of a damaged file alone. Every hint, of a damaged
    // file and of a missing one, followed through a shell from every directory of the shared
    // test matrix: the check passes again.
    withRepository(({ root, bench, follow }) => {
      const file = join(root, "spec", "b.json");
      for (const damage of [() => { writeFileSync(file, '{"render 64x36": 1000, "module": 500}'); }, () => { rmSync(file); }]) {
        for (const hint of ["git checkout HEAD -- :/spec/b.json", "cargo xtask bless-benchmarks"]) {
          damage();
          if (staged) {
            execFileSync("git", ["add", "spec"], { cwd: root, env: { ...hermeticGit(), HOME: root } });
          }
          const refused = bench("check");
          expect([refused.status, refused.hints], refused.line).toStrictEqual([1, ["git checkout HEAD -- :/spec/b.json", "cargo xtask bless-benchmarks"]]);
          follow(hint, directory);
          expect(bench("check"), hint).toStrictEqual({ status: 0, line: undefined, hints: [] });
        }
      }
    });
  });

  test("loses no untracked file of a directory in the baseline's place", () => {
    // Finding K13-3 of the sixth deep review of visual block 2 (#34): git deletes the directory, with
    // what it holds, to restore the file.
    withRepository(({ root, bench, follow }) => {
      const file = join(root, "spec", "b.json");
      rmSync(file);
      mkdirSync(file);
      writeFileSync(join(file, "untracked"), "kept");
      const { status, hints, line } = bench("check");
      expect([status, hints], line).toStrictEqual([1, ["pnpm --filter @poyetik/m benchmarks:check"]]);
      for (const hint of hints) {
        follow(hint, "");
        expect(readFileSync(join(file, "untracked"), "utf8"), hint).toBe("kept");
      }
      renameSync(file, join(root, "moved away"));
      follow(bench("check").hints[0] ?? "", "");
      expect(bench("check").status).toBe(0);
    });
  });
});

describe("what a library gives", () => {
  test("refuses a tolerance that cannot hold a count, and measurements that are no counts", () => {
    /** A measurement of render 64x36 of `value` instructions. */
    const render = (value: number): Measurement => ({ name: "render 64x36", value, unit: "instructions" });
    const cases: readonly (readonly [number, readonly Measurement[], string])[] = [
      [2.5, MEASURED, "error: the tolerance of dist/m.wasm's benchmarks, 2.5%, is not a whole percentage of 0 or more"],
      [Number.NaN, MEASURED, "error: the tolerance of dist/m.wasm's benchmarks, NaN%, is not a whole percentage of 0 or more"],
      [-1, MEASURED, "error: the tolerance of dist/m.wasm's benchmarks, -1%, is not a whole percentage of 0 or more"],
      [Number.POSITIVE_INFINITY, MEASURED, "error: the tolerance of dist/m.wasm's benchmarks, Infinity%, is not a whole percentage of 0 or more"],
      [2 ** 53, MEASURED, "error: the tolerance of dist/m.wasm's benchmarks, 9007199254740992%, is not a whole percentage of 0 or more"],
      [2, [render(1000), { name: "render 64x36", value: 500, unit: "bytes" }], "error: dist/m.wasm: the benchmark render 64x36 is measured twice"],
      [2, [render(0)], "error: dist/m.wasm: render 64x36 measured 0, not a whole count from 1 to 2^53 - 1"],
      [2, [render(1.5)], "error: dist/m.wasm: render 64x36 measured 1.5, not a whole count from 1 to 2^53 - 1"],
      [2, [render(-1)], "error: dist/m.wasm: render 64x36 measured -1, not a whole count from 1 to 2^53 - 1"],
      [2, [render(2 ** 53)], "error: dist/m.wasm: render 64x36 measured 9007199254740992, not a whole count from 1 to 2^53 - 1"],
    ];
    for (const [tolerance, measured, line] of cases) {
      for (const command of ["check", "bless"]) {
        const { io, err, written } = fakeIo(files());
        expect(benchMain(io, [command], { ...benchmarks(measured), tolerancePercent: tolerance }), line).toBe(1);
        expect(err, line).toStrictEqual([line]);
        expect(written.size, `${command}: nothing written`).toBe(0);
      }
    }
    // The edges accepted: a tolerance of 0 (every count exact), and the counts from 1 to
    // 2^53 - 1, which bless writes and the next check reads back as written (one rule,
    // isCount, for both).
    for (const count of [1, 2 ** 53 - 1]) {
      const blessed = fakeIo(files());
      expect(benchMain(blessed.io, ["bless"], { ...benchmarks([render(count)]), tolerancePercent: 0 }), `bless ${String(count)}`).toBe(0);
      const text = blessed.written.get("/repo/spec/b.json") ?? "";
      expect([blessed.err, text], `bless ${String(count)}`).toStrictEqual([[], `{\n  "render 64x36": ${String(count)}\n}\n`]);
      const checked = fakeIo(files(text));
      expect(benchMain(checked.io, ["check"], { ...benchmarks([render(count)]), tolerancePercent: 0 }), `check ${String(count)}`).toBe(0);
      expect(checked.err, `check ${String(count)}`).toStrictEqual([]);
    }
  });
});

describe("isCount", () => {
  test("holds a whole number from 1 to 2^53 - 1 and nothing else", () => {
    expect([1, 2, 2 ** 53 - 1].map(isCount)).toStrictEqual([true, true, true]);
    expect([0, -0, -1, 1.5, 2 ** 53, Number.NaN, Number.POSITIVE_INFINITY, "1", 1n, undefined].map(isCount)).toStrictEqual(Array<boolean>(10).fill(false));
  });
});

describe("the baseline", () => {
  test("reads the counts only as bless writes them, in the measurements' order", () => {
    expect(formatBaseline(MEASURED)).toBe(BLESSED);
    expect(parseBaseline(BLESSED)).toStrictEqual(new Map([["render 64x36", 1000], ["module", 500]]));
    expect(formatBaseline([])).toBe("{}\n");
    expect(() => parseBaseline('{"a": 1}\n')).toThrow(new Error("not as bless writes it (a name written twice, or another layout)"));
  });

  test("refuses what is not a JSON object of whole counts", () => {
    for (const [text, reason] of [
      ["[]\n", "not a JSON object of counts by benchmark name"],
      ["null\n", "not a JSON object of counts by benchmark name"],
      ["3\n", "not a JSON object of counts by benchmark name"],
      ['{\n  "a": -1\n}\n', "a: not a whole count from 1 to 2^53 - 1"],
      ['{\n  "a": 1.5\n}\n', "a: not a whole count from 1 to 2^53 - 1"],
      ['{\n  "a": "1"\n}\n', "a: not a whole count from 1 to 2^53 - 1"],
      ['{\n  "a": 9007199254740992\n}\n', "a: not a whole count from 1 to 2^53 - 1"],
      ['{"a": 1}', "not as bless writes it (a name written twice, or another layout)"],
    ] as const) {
      expect(() => parseBaseline(text), text).toThrow(new Error(reason));
    }
    for (const text of ["", "{"]) {
      expect(() => parseBaseline(text), text).toThrow(/^not JSON \(/u);
    }
    // A count of 0 is no measurement: a change from it would be infinite.
    expect(() => parseBaseline('{\n  "a": 0\n}\n')).toThrow(new Error("a: not a whole count from 1 to 2^53 - 1"));
    expect(parseBaseline('{\n  "a": 1\n}\n')).toStrictEqual(new Map([["a", 1]]));
    const notJson = ((): unknown => {
      try {
        return parseBaseline("{");
      } catch (error) {
        return error;
      }
    })();
    expect(notJson instanceof Error && notJson.cause instanceof SyntaxError).toBe(true);
  });
});

describe("regressions", () => {
  test("allows the tolerance either way, and no more", () => {
    const at = (value: number): string[] => regressions([{ name: "x", value, unit: "u" }], new Map([["x", 1000]]), 2);
    expect([at(980), at(1000), at(1020)]).toStrictEqual([[], [], []]);
    expect(at(1021)).toStrictEqual(["x: 1021 u, baseline 1000 (+2.10%, beyond ±2%)"]);
    expect(at(979)).toStrictEqual(["x: 979 u, baseline 1000 (-2.10%, beyond ±2%)"]);
  });

  test("compares exactly, where a product of numbers would round", () => {
    // 3 × 9 007 199 254 740 933 is above 2^53, where a number rounds it to a multiple of 4:
    // a difference of 270 215 977 642 228, a hundredth of it above 3%, reads within it as
    // numbers, beyond it in integers.
    const reference = 9_007_199_254_740_933;
    expect(regressions([{ name: "x", value: reference - 270_215_977_642_228, unit: "u" }], new Map([["x", reference]]), 3)).toStrictEqual(["x: 8736983277098705 u, baseline 9007199254740933 (-3.01%, beyond ±3%)"]);
    // A change beyond the tolerance never reads as within it: rounded away from zero.
    const beyond = (value: number, base: number, tolerance: number): string[] => regressions([{ name: "x", value, unit: "u" }], new Map([["x", base]]), tolerance);
    expect(beyond(102_001, 100_000, 2)).toStrictEqual(["x: 102001 u, baseline 100000 (+2.01%, beyond ±2%)"]);
    expect(beyond(103_010, 100_000, 3)).toStrictEqual(["x: 103010 u, baseline 100000 (+3.01%, beyond ±3%)"]);
    expect(beyond(97_999, 100_000, 2)).toStrictEqual(["x: 97999 u, baseline 100000 (-2.01%, beyond ±2%)"]);
    expect(beyond(205_000, 100_000, 2), "a whole percentage, two decimals").toStrictEqual(["x: 205000 u, baseline 100000 (+105.00%, beyond ±2%)"]);
    expect(() => beyond(1.5, 1, 2), "a count that is not whole").toThrow(new RangeError("The number 0.5 cannot be converted to a BigInt because it is not an integer"));
    expect(() => beyond(2, 1, 1.5), "a tolerance that is not whole").toThrow(new RangeError("The number 1.5 cannot be converted to a BigInt because it is not an integer"));
    expect(() => beyond(5, 0, 2), "a baseline of 0, which the command refuses").toThrow(new RangeError("Division by zero"));
    expect(regressions([{ name: "x", value: reference - 270_215_977_642_227, unit: "u" }], new Map([["x", reference]]), 3), "at the bound").toStrictEqual([]);
  });
});

describe("call", () => {
  test("calls an export with its arguments and returns its answer, and names one that is not a function", () => {
    const received: number[][] = [];
    const exports = {
      add: (first: number, second: number): number => {
        received.push([first, second]);
        return first + second;
      },
      memory: 3,
    };
    expect(call(exports, "add", 2, 5)).toBe(7);
    expect(received).toStrictEqual([[2, 5]]);
    expect(() => call(exports, "memory")).toThrow(new TypeError("no function memory"));
    expect(() => call(exports, "absent")).toThrow(new TypeError("no function absent"));
  });
});

describe("counting", () => {
  test("stores a count exactly up to 2^53 - 1, and refuses more", () => {
    expect(exact(9_007_199_254_740_991n)).toBe(9_007_199_254_740_991);
    expect(() => exact(9_007_199_254_740_992n)).toThrow(new RangeError("a count of 9007199254740992 cannot be stored exactly"));
  });

  test("names the benchmark whose measurement failed, keeping the cause", () => {
    expect(forBenchmark("b", () => 3)).toBe(3);
    const cause = new Error("refused");
    expect(() => forBenchmark("b", () => { throw cause; })).toThrow(expect.objectContaining({ message: "b: refused", cause }));
    expect(() => forBenchmark("b", () => { throw new RangeError("deep"); })).toThrow(new Error("b: deep"));
  });

  test("counts what an action executes in an instrumented instance", () => {
    // `i32.const 1; drop; end`: two instructions, the structure marker `end` uncounted.
    const module = new WebAssembly.Module(instrument(assemble({ functions: [{ params: 0, result: false, locals: [0], code: [0x41, 0x01, 0x1a, 0x0b], exported: "f" }] })));
    const { exports, counted } = countedInstance(module);
    const run: unknown = exports["f"];
    if (typeof run !== "function") {
      throw new TypeError("no function f");
    }
    expect(counted(() => { Reflect.apply(run, undefined, []); })).toBe(2n);
    expect(counted(() => undefined)).toBe(0n);
  });

  test("refuses a module without its counter, and a counter that is not an i64", () => {
    const plain = new WebAssembly.Module(assemble({ functions: [], globals: [I32_GLOBAL], exports: [["instructions", 0x03, 0]] }));
    expect(() => countedInstance(new WebAssembly.Module(assemble({ functions: [] })))).toThrow(new Error('the module does not export its counter "instructions"'));
    const wrong = countedInstance(plain);
    expect(() => wrong.counted(() => undefined)).toThrow(new Error('the counter "instructions" is not an i64'));
    expect(counterValue(7n)).toBe(7n);
  });
});

test("reports a failure that is not an Error by its text", () => {
  const { io, err } = fakeIo(files());
  const failing: BenchIo = {
    ...io,
    read: () => {
      // A generator rethrows what it is given: a value that is not an Error, thrown as a
      // misbehaving callee could.
      const thrower = (function* (): Generator<number> {
        yield 0;
      })();
      thrower.next();
      thrower.throw("disk gone");
      return Uint8Array.of();
    },
  };
  expect(benchMain(failing, ["check"], benchmarks())).toBe(1);
  expect(err).toStrictEqual(["error: cannot read spec/b.json: disk gone"]);
});
