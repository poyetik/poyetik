import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, test } from "vitest";
import { BASELINE_FILE, benchMain, MODULE_FILE, SUITE, THRESHOLDS_FILE, USAGE, type BenchIo, type Suite } from "../../benchmarks/cli.ts";
import { BOX_BENCHMARKS, MIX_BENCHMARKS, PLAN_SIZES } from "../../benchmarks/suite.ts";
import { rethrow, wasmBytes } from "../fixtures.ts";
import { assemble } from "./assemble.ts";

/** The thresholds of the tests: a tolerance of 2%. */
const THRESHOLDS = JSON.stringify({
  tolerance_percent: 2,
  construction: { runs: 1, margin: 1, measured_on: "now", measured_max_ms: { chromium: 1, firefox: 1, webkit: 1 } },
  boxes: { blocks: 2, runs: 1, margin: 1, measured_on: "now", measured_max_ms: { chromium: 1, firefox: 1, webkit: 1 } },
});

/** A small suite: one box benchmark, one mixing benchmark, two plans. */
const SMALL: Suite = { boxes: BOX_BENCHMARKS.slice(0, 1), mixes: MIX_BENCHMARKS.slice(0, 1), sizes: [3, 5] };

/** A fake outside world over files in memory, recording what is printed and written. */
class Fake implements BenchIo {
  /** The repository root. */
  readonly repository = "/repo";
  /** The files by path: bytes, or what reading them throws. */
  readonly files = new Map<string, Uint8Array | Error | string>();
  /** The lines printed on stdout. */
  readonly printed: string[] = [];
  /** The lines printed on stderr. */
  readonly errors: string[] = [];
  /** What is thrown on a write, if anything. */
  writeFails: unknown;

  /**
   * Starts with the thresholds and the built module.
   * @param baseline The baseline file, if any.
   */
  constructor(baseline?: string) {
    this.files.set(`/repo/${THRESHOLDS_FILE}`, new TextEncoder().encode(THRESHOLDS));
    this.files.set(`/repo/${MODULE_FILE}`, new Uint8Array(wasmBytes()));
    if (baseline !== undefined) {
      this.files.set(`/repo/${BASELINE_FILE}`, new TextEncoder().encode(baseline));
    }
  }

  /** Reads a file of {@link Fake.files}. */
  readonly read = (path: string): Uint8Array => {
    const file = this.files.get(path);
    if (file === undefined) {
      throw new Error(`ENOENT: ${path}`);
    }
    if (file instanceof Error) {
      throw file;
    }
    if (typeof file === "string") {
      rethrow(file);
    }
    return file;
  };

  /** Writes a file of {@link Fake.files}, unless {@link Fake.writeFails}. */
  readonly write = (path: string, text: string): void => {
    if (this.writeFails !== undefined) {
      rethrow(this.writeFails);
    }
    this.files.set(path, new TextEncoder().encode(text));
  };

  /** Records a line printed on stdout. */
  readonly out = (line: string): void => {
    this.printed.push(line);
  };

  /** Records a line printed on stderr. */
  readonly err = (line: string): void => {
    this.errors.push(line);
  };

  /** A file written, as text. */
  text(path: string): string {
    const file = this.files.get(path);
    return file instanceof Uint8Array ? new TextDecoder().decode(file) : "";
  }
}

/** Blesses a fresh fake with `suite`, and returns it. */
function blessed(suite: Suite = SMALL): Fake {
  const fake = new Fake();
  expect(benchMain(fake, ["bless"], suite)).toBe(0);
  return fake;
}

describe("benchMain", () => {
  test("blesses a baseline of every count, then checks it", () => {
    const fake = blessed();
    expect(fake.errors).toStrictEqual([]);
    expect(fake.printed).toHaveLength(8);
    expect(fake.printed[0]).toMatch(/^osc~ 440 Hz at 48 kHz in binary64: \d+ instructions$/);
    expect(fake.printed[1]).toMatch(/^osc~ 440 Hz at 48 kHz in binary32: \d+ instructions$/);
    expect(fake.printed.slice(2, 6).map((line) => line.replace(/\d+ instructions/, "N"))).toStrictEqual([
      "mixing 2 cables into one inlet at 48 kHz in binary64: N",
      "mixing 2 cables into one inlet at 48 kHz in binary32: N",
      "loading a plan of 3 oscillators: N",
      "loading a plan of 5 oscillators: N",
    ]);
    expect(fake.printed[6]).toBe(`engine module: ${String(wasmBytes().byteLength)} bytes`);
    expect(fake.printed[7]).toBe("wrote spec/benchmark-baseline.json");
    const baseline: unknown = JSON.parse(fake.text(`/repo/${BASELINE_FILE}`));
    expect(Reflect.get(Object(baseline), "mixes")).toHaveLength(2);
    expect(Reflect.get(Object(baseline), "plans")).toHaveLength(2);
    fake.printed.length = 0;
    expect(benchMain(fake, ["check"], SMALL)).toBe(0);
    expect(fake.printed).toHaveLength(7);
    expect(fake.errors).toStrictEqual([]);
  });

  test("fails a check beyond the tolerance, naming each benchmark, with one error line", () => {
    const fake = blessed();
    const path = `/repo/${BASELINE_FILE}`;
    fake.files.set(path, new TextEncoder().encode(fake.text(path).replace(/"instructions": (\d+)/, (_all, count: string) => `"instructions": ${String(Math.round(Number(count) * 1.03))}`)));
    expect(benchMain(fake, ["check"], SMALL)).toBe(1);
    expect(fake.printed.at(-1)).toMatch(/^osc~ 440 Hz at 48 kHz in binary64: \d+ instructions, baseline \d+ \(-2\.9\d%, beyond ±2%\)$/);
    expect(fake.errors).toStrictEqual(["error: the benchmarks of bindings/wasm/dist/engine.wasm moved beyond the thresholds of spec/benchmark-thresholds.json from spec/benchmark-baseline.json (1 problem(s); see spec/benchmarks.md)"]);
  });

  test("fails a check when the engine module's size moved beyond the tolerance", () => {
    const fake = blessed();
    const path = `/repo/${BASELINE_FILE}`;
    const size = wasmBytes().byteLength;
    fake.files.set(path, new TextEncoder().encode(fake.text(path).replace(`"module_bytes": ${String(size)}`, `"module_bytes": ${String(Math.round(size / 1.03))}`)));
    expect(benchMain(fake, ["check"], SMALL)).toBe(1);
    expect(fake.printed.at(-1)).toMatch(new RegExp(`^engine module: ${String(size)} bytes, baseline \\d+ \\(\\+3\\.0\\d%, beyond ±2%\\)$`));
  });

  test("fails a check, and refuses to bless, when plans break the n log n law", () => {
    const decreasing: Suite = { boxes: [], mixes: [], sizes: [5, 3] };
    const fake = blessed({ boxes: [], mixes: [], sizes: [3] });
    expect(benchMain(fake, ["check"], decreasing)).toBe(1);
    expect(fake.printed.filter((line) => line === "the n log n law needs plans in increasing size: 3 oscillators after 5")).toHaveLength(1);
    expect(fake.errors).toStrictEqual(["error: the benchmarks of bindings/wasm/dist/engine.wasm moved beyond the thresholds of spec/benchmark-thresholds.json from spec/benchmark-baseline.json (2 problem(s); see spec/benchmarks.md)"]);
    const before = fake.text(`/repo/${BASELINE_FILE}`);
    fake.errors.length = 0;
    expect(benchMain(fake, ["bless"], decreasing)).toBe(1);
    expect(fake.errors).toStrictEqual(["error: no spec/benchmark-baseline.json written: the plans of bindings/wasm/dist/engine.wasm break the n log n law (1 problem(s); see spec/benchmarks.md)"]);
    expect(fake.text(`/repo/${BASELINE_FILE}`)).toBe(before);
  });

  test("names the file it cannot read, or whose contents are wrong", () => {
    const missing = new Fake();
    expect(benchMain(missing, ["check"], SMALL)).toBe(1);
    expect(missing.errors).toStrictEqual(["error: cannot read spec/benchmark-baseline.json: ENOENT: /repo/spec/benchmark-baseline.json"]);
    const malformed = new Fake("{}");
    expect(benchMain(malformed, ["check"], SMALL)).toBe(1);
    expect(malformed.errors).toStrictEqual(['error: spec/benchmark-baseline.json: expected {"boxes": [...], "mixes": [...], "plans": [...], "module_bytes": <whole>}']);
    const thresholds = new Fake("{}");
    thresholds.files.set(`/repo/${THRESHOLDS_FILE}`, Uint8Array.of(0xff));
    expect(benchMain(thresholds, ["check"], SMALL)).toBe(1);
    expect(thresholds.errors[0]).toMatch(/^error: cannot read spec\/benchmark-thresholds\.json: .+/);
    const noModule = new Fake();
    noModule.files.delete(`/repo/${MODULE_FILE}`);
    expect(benchMain(noModule, ["bless"], SMALL)).toBe(1);
    expect(noModule.errors).toStrictEqual(["error: cannot read bindings/wasm/dist/engine.wasm: ENOENT: /repo/bindings/wasm/dist/engine.wasm"]);
  });

  test("names the module when it cannot be instrumented, compiled or driven, or a box costs less than nothing", () => {
    const run = (module: Uint8Array, suite: Suite = SMALL): string[] => {
      const fake = new Fake();
      fake.files.set(`/repo/${MODULE_FILE}`, module);
      expect(benchMain(fake, ["bless"], suite)).toBe(1);
      return fake.errors;
    };
    expect(run(Uint8Array.of(1, 2, 3))).toStrictEqual(["error: bindings/wasm/dist/engine.wasm: not a WebAssembly module of version 1"]);
    const invalid = assemble({ functions: [{ params: 0, result: false, locals: [0x00], code: [0x6a, 0x0b], exported: "f" }] });
    expect(run(invalid)[0]).toMatch(/^error: bindings\/wasm\/dist\/engine\.wasm: WebAssembly\.Module\(\): .+/);
    const empty = assemble({ functions: [{ params: 0, result: false, locals: [0x00], code: [0x0b], exported: "f" }] });
    expect(run(empty)).toStrictEqual(['error: bindings/wasm/dist/engine.wasm: osc~ 440 Hz at 48 kHz in binary64: not an engine module: no memory']);
    const [first] = BOX_BENCHMARKS;
    if (first === undefined) {
      throw new Error("no benchmark");
    }
    expect(run(new Uint8Array(wasmBytes()), { boxes: [{ ...first, patch: first.without, without: first.patch }], mixes: [], sizes: [] })[0]).toMatch(
      /^error: bindings\/wasm\/dist\/engine\.wasm: osc~ 440 Hz at 48 kHz in binary64: the patch without it costs \d+ instructions more than the patch holding it$/,
    );
  });

  test("names the baseline when it cannot write it, whatever was thrown", () => {
    for (const [thrown, words] of [
      [new Error("EACCES"), "EACCES"],
      ["read-only", "read-only"],
    ] as const) {
      const fake = new Fake();
      fake.writeFails = thrown;
      expect(benchMain(fake, ["bless"], SMALL)).toBe(1);
      expect(fake.errors).toStrictEqual([`error: cannot write spec/benchmark-baseline.json: ${words}`]);
    }
  });

  test("tells a thrown value that is not an error in words", () => {
    const fake = new Fake("{}");
    fake.files.set(`/repo/${THRESHOLDS_FILE}`, "busy");
    expect(benchMain(fake, ["check"], SMALL)).toBe(1);
    expect(fake.errors).toStrictEqual(["error: cannot read spec/benchmark-thresholds.json: busy"]);
  });

  test("prints its usage on help only, and refuses usage errors in one line", () => {
    const help = new Fake();
    expect(benchMain(help, ["help"])).toBe(0);
    expect([help.printed, help.errors]).toStrictEqual([[USAGE], []]);
    expect(USAGE.split("\n")[0]).toBe("usage: node scripts/benchmarks.ts <check|bless|help> (in bindings/wasm)");
    for (const [args, reason] of [
      [[], "no command given"],
      [["nope"], 'unknown command "nope"'],
      [["nope", "x"], 'unknown command "nope"'],
      [["check", "x"], "`check` takes no argument"],
      [["bless", "x"], "`bless` takes no argument"],
      [["help", "x"], "`help` takes no argument"],
      [["\u001b[2Jx"], 'unknown command "\\u{1b}[2Jx"'],
    ] as const) {
      const fake = new Fake();
      expect(benchMain(fake, args)).toBe(2);
      expect([fake.printed, fake.errors]).toStrictEqual([[], [`error: ${reason} (see: node scripts/benchmarks.ts help)`]]);
    }
  });

  test("names, in a usage error, a command that runs as written in the package and prints the usage", () => {
    const fake = new Fake();
    expect(benchMain(fake, [])).toBe(2);
    const hint = /\(see: (.+)\)$/.exec(fake.errors[0] ?? "")?.[1];
    expect(hint).toBeDefined();
    const [program = "", ...args] = (hint ?? "").split(" ");
    const ran = spawnSync(program, args, { cwd: fileURLToPath(new URL("../..", import.meta.url)), encoding: "utf8" });
    expect([ran.status, ran.stdout, ran.stderr]).toStrictEqual([0, `${USAGE}\n`, ""]);
  });

  test("escapes what it prints of a problem", () => {
    const [first] = BOX_BENCHMARKS;
    if (first === undefined) {
      throw new Error("no benchmark");
    }
    const fake = blessed({ boxes: [{ ...first, setting: "a\nb" }], mixes: [], sizes: [] });
    expect(benchMain(fake, ["check"], SMALL)).toBe(1);
    expect(fake.printed).toContain("osc~ a\\u{a}b in binary64: in the baseline, measured by no benchmark");
    expect(fake.printed).toContain("osc~ a\\u{a}b in binary32: in the baseline, measured by no benchmark");
  });

  test("runs the whole suite by default, as the paths of the repository say", () => {
    expect(SUITE).toStrictEqual({ boxes: BOX_BENCHMARKS, mixes: MIX_BENCHMARKS, sizes: PLAN_SIZES });
    expect([BASELINE_FILE, THRESHOLDS_FILE, MODULE_FILE]).toStrictEqual(["spec/benchmark-baseline.json", "spec/benchmark-thresholds.json", "bindings/wasm/dist/engine.wasm"]);
    expect(readFileSync(new URL("../../package.json", import.meta.url), "utf8")).toMatch(/"benchmarks:check": "[a-z]+ scripts\/benchmarks\.ts check"/);
  });
});
