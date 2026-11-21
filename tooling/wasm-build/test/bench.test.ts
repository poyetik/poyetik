import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
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
        throw new Error(`ENOENT ${path}`);
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
    buildCommand: "pnpm build:wasm",
    command: "node scripts/benchmarks.ts",
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

describe("benchMain", () => {
  test("refuses no command, another command and an argument, as usage errors", () => {
    for (const [args, line] of [
      [[], "error: no command given (see: node scripts/benchmarks.ts help)"],
      [["run"], 'error: unknown command "run" (see: node scripts/benchmarks.ts help)'],
      [["check", "now"], "error: `check` takes no argument (see: node scripts/benchmarks.ts help)"],
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
      "usage: node scripts/benchmarks.ts <check|bless|help>\n  check  measure dist/m.wasm and compare it with spec/b.json, within ±2%, and with its laws\n  bless  measure it and rewrite spec/b.json, if its laws hold\n  help   print this help",
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
      "render 64x36: 1000 instructions, baseline 900 (+11.11%, beyond ±2%)",
      "module: 500 bytes, not in the baseline",
      "gone: in the baseline, measured by no benchmark",
    ]);
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

  test("names the file it cannot read, the module it cannot measure, and a baseline not as bless writes it", () => {
    for (const [given, measured, args, line] of [
      [{}, MEASURED, ["check"], "error: cannot read spec/b.json: ENOENT /repo/spec/b.json"],
      [{ "/repo/spec/b.json": new TextEncoder().encode(BLESSED) }, MEASURED, ["check"], "error: cannot read dist/m.wasm (build it: pnpm build:wasm): ENOENT /repo/dist/m.wasm"],
      [{ ...files(), "/repo/spec/b.json": Uint8Array.of(0xff) }, MEASURED, ["check"], "error: cannot read spec/b.json: The encoded data was not valid for encoding utf-8"],
      [files('{"module": 500}'), MEASURED, ["check"], "error: spec/b.json: not as bless writes it (a name written twice, or another layout): restore it (see: git checkout -- :/spec/b.json)"],
      [files("[]"), MEASURED, ["check"], "error: spec/b.json: not a JSON object of counts by benchmark name: restore it (see: git checkout -- :/spec/b.json)"],
      [files(), new Error("render 64x36: refused"), ["bless"], "error: dist/m.wasm: render 64x36: refused"],
    ] as const) {
      const { io, err } = fakeIo(given);
      expect(benchMain(io, args, benchmarks(measured)), line).toBe(1);
      expect(err, line).toStrictEqual([line]);
    }
  });
});

describe("the restore a check names", () => {
  test("restores the baseline as committed when run as written, from the package's directory too", () => {
    const root = mkdtempSync(join(tmpdir(), "bench-restore-"));
    try {
      /** Runs git in `cwd` with no user's or system's configuration, a committer of its own. */
      const git = (cwd: string, args: readonly string[]): void => {
        execFileSync("git", ["-c", "user.name=bench", "-c", "user.email=bench@example.invalid", "-c", "commit.gpgsign=false", ...args], {
          cwd,
          env: { ...process.env, GIT_CONFIG_GLOBAL: devNull, GIT_CONFIG_NOSYSTEM: "1" },
          stdio: "pipe",
        });
      };
      for (const directory of ["spec", "dist", "package"]) {
        mkdirSync(join(root, directory));
      }
      writeFileSync(join(root, "dist", "m.wasm"), Uint8Array.of(0, 97, 115, 109));
      writeFileSync(join(root, "spec", "b.json"), BLESSED);
      writeFileSync(join(root, "package", "README.md"), "a package\n");
      git(root, ["init", "-q"]);
      git(root, ["add", "."]);
      git(root, ["commit", "-q", "-m", "baseline"]);
      writeFileSync(join(root, "spec", "b.json"), '{"render 64x36": 1000, "module": 500}');
      const err: string[] = [];
      const io: BenchIo = { repository: root, read: (path) => new Uint8Array(readFileSync(path)), write: () => undefined, out: () => undefined, err: (line) => err.push(line) };
      expect(benchMain(io, ["check"], benchmarks())).toBe(1);
      const hint = /\(see: git (?<command>[^)]+)\)$/u.exec(err[0] ?? "")?.groups?.["command"];
      expect(hint, err[0]).toBe("checkout -- :/spec/b.json");
      git(join(root, "package"), (hint ?? "").split(" "));
      err.length = 0;
      expect([benchMain(io, ["check"], benchmarks()), err]).toStrictEqual([0, []]);
    } finally {
      rmSync(root, { recursive: true });
    }
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
  test("reads back what it writes, in the measurements' order", () => {
    expect(formatBaseline(MEASURED)).toBe(BLESSED);
    expect(parseBaseline(BLESSED)).toStrictEqual(new Map([["render 64x36", 1000], ["module", 500]]));
    expect(formatBaseline([])).toBe("{}\n");
  });

  test("refuses what is not a JSON object of whole counts", () => {
    for (const [text, reason] of [
      ["", /^not JSON \(/u],
      ["{", /^not JSON \(/u],
      ["[]\n", /^not a JSON object of counts by benchmark name$/u],
      ["null\n", /^not a JSON object of counts by benchmark name$/u],
      ["3\n", /^not a JSON object of counts by benchmark name$/u],
      ['{\n  "a": -1\n}\n', /^a: not a whole count from 1 to 2\^53 - 1$/u],
      ['{\n  "a": 1.5\n}\n', /^a: not a whole count from 1 to 2\^53 - 1$/u],
      ['{\n  "a": "1"\n}\n', /^a: not a whole count from 1 to 2\^53 - 1$/u],
      ['{\n  "a": 9007199254740992\n}\n', /^a: not a whole count from 1 to 2\^53 - 1$/u],
      ['{"a": 1}', /^not as bless writes it/u],
    ] as const) {
      expect(() => parseBaseline(text), text).toThrow(reason);
    }
    // A count of 0 is no measurement: a change from it would be infinite.
    expect(() => parseBaseline('{\n  "a": 0\n}\n')).toThrow(/^a: not a whole count from 1 to 2\^53 - 1$/u);
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
    expect(regressions([{ name: "x", value: reference - 270_215_977_642_228, unit: "u" }], new Map([["x", reference]]), 3)).toStrictEqual(["x: 8736983277098705 u, baseline 9007199254740933 (-3.00%, beyond ±3%)"]);
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
