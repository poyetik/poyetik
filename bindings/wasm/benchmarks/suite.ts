// The benchmarks of the engine, counted in WebAssembly instructions (spec/benchmarks.md):
// what each measures, the baseline they are compared with, the tolerance, and the law that
// building a plan grows as n log n. Pure functions over a counted module, tested.
import { CountedInstance } from "./counter.ts";

/** A patch in the patch format, before it is written as JSON. */
interface Patch {
  /** The format version. */
  readonly format: 1;
  /** The boxes. */
  readonly nodes: readonly { readonly id: number; readonly box: string; readonly params?: Readonly<Record<string, number>> }[];
  /** The cables. */
  readonly connections: readonly { readonly from: readonly [number, number]; readonly to: readonly [number, number] }[];
}

/**
 * The cost of one box at one setting: the instructions of {@link BLOCKS} blocks of a
 * patch holding it, minus those of the same blocks of the same patch without it. So it
 * counts the box's `process` and what the plan does for it each block (gathering its
 * inputs, copying its outputs), which is what a patch pays for it.
 */
export interface BoxBenchmark {
  /** The box type measured, as patches name it. */
  readonly box: string;
  /** The setting, in words: parameters, rate, connections. */
  readonly setting: string;
  /** The sample rate, in hertz. */
  readonly rate: number;
  /** A patch holding the box. */
  readonly patch: Patch;
  /** The same patch without it. */
  readonly without: Patch;
}

/** Blocks rendered per box benchmark: enough for every phase of a slow sine to vary its cost. */
export const BLOCKS = 64;

/**
 * Every box benchmark: each box type of the engine at representative settings (low and
 * high rates and frequencies, a constant sine; an output fed or not). Adding a box type
 * adds its benchmarks here (spec/benchmarks.md, "Adding a benchmark"). The patches are
 * written out whole, so that each is read as it is.
 */
export const BOX_BENCHMARKS: readonly BoxBenchmark[] = [
  {
    box: "osc~",
    setting: "440 Hz at 48 kHz",
    rate: 48_000,
    patch: { format: 1, nodes: [{ id: 1, box: "osc~", params: { frequency: 440 } }], connections: [] },
    without: { format: 1, nodes: [], connections: [] },
  },
  {
    box: "osc~",
    setting: "1000 Hz at 8 kHz",
    rate: 8000,
    patch: { format: 1, nodes: [{ id: 1, box: "osc~", params: { frequency: 1000 } }], connections: [] },
    without: { format: 1, nodes: [], connections: [] },
  },
  {
    box: "osc~",
    setting: "19999 Hz at 384 kHz",
    rate: 384_000,
    patch: { format: 1, nodes: [{ id: 1, box: "osc~", params: { frequency: 19_999 } }], connections: [] },
    without: { format: 1, nodes: [], connections: [] },
  },
  {
    box: "osc~",
    setting: "0 Hz at 44.1 kHz",
    rate: 44_100,
    patch: { format: 1, nodes: [{ id: 1, box: "osc~", params: { frequency: 0 } }], connections: [] },
    without: { format: 1, nodes: [], connections: [] },
  },
  {
    box: "dac~",
    setting: "two inlets fed at 48 kHz",
    rate: 48_000,
    patch: {
      format: 1,
      nodes: [
        { id: 1, box: "osc~", params: { frequency: 440 } },
        { id: 2, box: "dac~" },
      ],
      connections: [
        { from: [1, 0], to: [2, 0] },
        { from: [1, 0], to: [2, 1] },
      ],
    },
    without: { format: 1, nodes: [{ id: 1, box: "osc~", params: { frequency: 440 } }], connections: [] },
  },
  {
    box: "dac~",
    setting: "unconnected at 48 kHz",
    rate: 48_000,
    patch: { format: 1, nodes: [{ id: 1, box: "dac~" }], connections: [] },
    without: { format: 1, nodes: [], connections: [] },
  },
];

/** The sample rate the plans are built at, in hertz. */
export const PLAN_RATE = 48_000;

/** The sizes of the plans built, in oscillators: each four times the previous. */
export const PLAN_SIZES: readonly number[] = [256, 1024, 4096, 16_384];

/**
 * A patch of `size` oscillators (ids 1 to `size`, at different frequencies) feeding the
 * two inlets of one `dac~` (id 0) in turn: `size + 1` boxes and `size` cables.
 */
export function planPatch(size: number): Patch {
  return {
    format: 1,
    nodes: [{ id: 0, box: "dac~" }, ...Array.from({ length: size }, (_entry, index) => ({ id: index + 1, box: "osc~", params: { frequency: 100 + (index % 1000) } }))],
    connections: Array.from({ length: size }, (_entry, index) => ({ from: [index + 1, 0] as const, to: [0, index % 2] as const })),
  };
}

/** The bytes of a patch. */
export function patchBytes(patch: Patch): Uint8Array {
  return new TextEncoder().encode(JSON.stringify(patch));
}

/** What one run of the benchmarks measured, in the shape of the baseline. */
export interface Counts {
  /** Each box benchmark: box, setting and instructions. */
  readonly boxes: readonly { readonly box: string; readonly setting: string; readonly instructions: number }[];
  /** Each plan built: its size in oscillators, and the instructions of loading it. */
  readonly plans: readonly { readonly oscillators: number; readonly instructions: number }[];
}

/**
 * A count as a number, which JSON can hold exactly.
 * @throws {RangeError} when it is negative or beyond 2^53.
 */
function exact(count: bigint): number {
  const value = Number(count);
  if (count < 0n || !Number.isSafeInteger(value)) {
    throw new RangeError(`a count of ${String(count)} instructions cannot be stored exactly`);
  }
  return value;
}

/** The instructions of {@link BLOCKS} blocks of `patch`, in a fresh instance. */
function blocksOf(module: WebAssembly.Module, patch: Patch, rate: number): bigint {
  const instance = new CountedInstance(module);
  instance.load(patchBytes(patch), rate);
  return instance.process(BLOCKS);
}

/**
 * Runs the benchmarks on a counted engine module: `boxes`, then plans of `sizes`.
 * @throws {CountError} when the engine refuses a patch.
 * @throws {RangeError} when a box costs less than nothing (the patch without it costs more).
 */
export function measure(module: WebAssembly.Module, boxes: readonly BoxBenchmark[], sizes: readonly number[]): Counts {
  return {
    boxes: boxes.map((benchmark) => ({
      box: benchmark.box,
      setting: benchmark.setting,
      instructions: exact(blocksOf(module, benchmark.patch, benchmark.rate) - blocksOf(module, benchmark.without, benchmark.rate)),
    })),
    plans: sizes.map((oscillators) => ({ oscillators, instructions: exact(new CountedInstance(module).load(patchBytes(planPatch(oscillators)), PLAN_RATE)) })),
  };
}

/** The name of a box benchmark in the reports. */
function boxName(entry: { readonly box: string; readonly setting: string }): string {
  return `${entry.box} ${entry.setting}`;
}

/** The name of a plan benchmark in the reports. */
function planName(oscillators: number): string {
  return `loading a plan of ${String(oscillators)} oscillators`;
}

/** Every benchmark of `counts`, by name, with its instructions. */
function named(counts: Counts): Map<string, number> {
  return new Map([
    ...counts.boxes.map((entry): [string, number] => [boxName(entry), entry.instructions]),
    ...counts.plans.map((entry): [string, number] => [planName(entry.oscillators), entry.instructions]),
  ]);
}

/** A change in percent with two decimals, signed when it is not zero: `+2.10`, `-0.50`, `0.00`. */
export function signedPercent(change: number): string {
  return `${change > 0 ? "+" : ""}${change.toFixed(2)}`;
}

/**
 * What differs between `measured` and `baseline` beyond `tolerance` percent of the
 * baseline, in either direction (a cost that fell is a baseline to bless, so that it
 * stays tight), and every benchmark found in only one of them. Empty when they agree.
 */
export function regressions(measured: Counts, baseline: Counts, tolerance: number): string[] {
  const now = named(measured);
  const before = named(baseline);
  const problems: string[] = [];
  for (const [name, instructions] of now) {
    const reference = before.get(name);
    if (reference === undefined) {
      problems.push(`${name}: ${String(instructions)} instructions, not in the baseline`);
    } else if (Math.abs(instructions - reference) * 100 > reference * tolerance) {
      const change = signedPercent(((instructions - reference) * 100) / reference);
      problems.push(`${name}: ${String(instructions)} instructions, baseline ${String(reference)} (${change}%, beyond ±${String(tolerance)}%)`);
    }
  }
  for (const name of before.keys()) {
    if (!now.has(name)) {
      problems.push(`${name}: in the baseline, measured by no benchmark`);
    }
  }
  return problems;
}

/**
 * Where the cost of building plans grows faster than n log n: for each plan and the next
 * larger one, the ratio of their costs must not exceed the ratio of n log2 n. A cost
 * a·n log n + b·n + c with non-negative terms always passes; a quadratic term fails as
 * soon as it dominates. Plans must be sorted by size, each larger than 1.
 */
export function nLogNProblems(plans: Counts["plans"]): string[] {
  const problems: string[] = [];
  const growth = (size: number): number => size * Math.log2(size);
  let smaller: Counts["plans"][number] | undefined;
  for (const larger of plans) {
    if (smaller !== undefined) {
      const allowed = growth(larger.oscillators) / growth(smaller.oscillators);
      const ratio = larger.instructions / smaller.instructions;
      if (ratio > allowed) {
        problems.push(
          `building plans grows faster than n log n: ${String(smaller.oscillators)} to ${String(larger.oscillators)} oscillators costs ${ratio.toFixed(3)} times more, at most ${allowed.toFixed(3)}`,
        );
      }
    }
    smaller = larger;
  }
  return problems;
}

/** Thrown when a benchmark file is malformed: one line naming what is wrong. */
export class BenchmarkFileError extends Error {
  /**
   * Describes what is wrong.
   * @param message What is wrong.
   */
  constructor(message: string) {
    super(message);
    this.name = "BenchmarkFileError";
  }
}

/** Whether `value` is a plain JSON object with exactly the keys `keys`, in any order. */
function hasExactly(value: unknown, keys: readonly string[]): value is Record<string, unknown> {
  // A copy of a primitive or a list has no key or only indices: never `keys`, which are names.
  return Object.keys(Object.assign({}, value)).sort().join() === [...keys].sort().join();
}

/** Whether `value` is a whole number of instructions JSON holds exactly. */
function isCount(value: unknown): value is number {
  return Number.isSafeInteger(value) && Number(value) >= 0;
}

/**
 * Reads a baseline (spec/benchmark-baseline.json), refusing anything but its exact shape.
 * @throws {BenchmarkFileError} naming what is wrong.
 */
export function parseBaseline(text: string): Counts {
  let json: unknown;
  try {
    json = JSON.parse(text);
  } catch (error) {
    throw new BenchmarkFileError(`not JSON: ${String(error)}`);
  }
  if (!hasExactly(json, ["boxes", "plans"]) || !Array.isArray(json["boxes"]) || !Array.isArray(json["plans"])) {
    throw new BenchmarkFileError('expected {"boxes": [...], "plans": [...]}');
  }
  const boxes = json["boxes"].map((entry: unknown, index) => {
    if (!hasExactly(entry, ["box", "setting", "instructions"]) || typeof entry["box"] !== "string" || typeof entry["setting"] !== "string" || !isCount(entry["instructions"])) {
      throw new BenchmarkFileError(`boxes[${String(index)}] is not {"box", "setting", "instructions"} with a whole count`);
    }
    return { box: entry["box"], setting: entry["setting"], instructions: entry["instructions"] };
  });
  const plans = json["plans"].map((entry: unknown, index) => {
    if (!hasExactly(entry, ["oscillators", "instructions"]) || !isCount(entry["oscillators"]) || !isCount(entry["instructions"])) {
      throw new BenchmarkFileError(`plans[${String(index)}] is not {"oscillators", "instructions"} with whole numbers`);
    }
    return { oscillators: entry["oscillators"], instructions: entry["instructions"] };
  });
  return { boxes, plans };
}

/** Writes a baseline as its file holds it: two-space indents, a final line break. */
export function formatBaseline(counts: Counts): string {
  return `${JSON.stringify({ boxes: counts.boxes, plans: counts.plans }, null, 2)}\n`;
}

/** The browser engines the construction of the engine processor is timed on, as Playwright names them. */
export const BROWSERS = ["chromium", "firefox", "webkit"] as const;

/** A browser engine of {@link BROWSERS}. */
export type Browser = (typeof BROWSERS)[number];

/** The thresholds of the benchmarks (spec/benchmark-thresholds.json), derived in spec/benchmarks.md. */
export interface Thresholds {
  /** How far, in percent of the baseline, an instruction count may move either way. */
  readonly tolerancePercent: number;
  /** How the construction of the engine processor is bounded. */
  readonly construction: {
    /** Constructions timed per browser engine and run. */
    readonly runs: number;
    /** The factor between the slowest construction measured and the bound. */
    readonly margin: number;
    /** Where and when the measured values were taken. */
    readonly measuredOn: string;
    /** The slowest of the constructions measured, per browser engine, in milliseconds. */
    readonly measuredMaxMs: Readonly<Record<Browser, number>>;
  };
}

/** Whether `value` is a positive, finite number. */
function isPositive(value: unknown): value is number {
  return Number.isFinite(value) && Number(value) > 0;
}

/**
 * Reads the thresholds file, refusing anything but its exact shape.
 * @throws {BenchmarkFileError} naming what is wrong.
 */
export function parseThresholds(text: string): Thresholds {
  let json: unknown;
  try {
    json = JSON.parse(text);
  } catch (error) {
    throw new BenchmarkFileError(`not JSON: ${String(error)}`);
  }
  if (!hasExactly(json, ["tolerance_percent", "construction"]) || !isPositive(json["tolerance_percent"])) {
    throw new BenchmarkFileError('expected {"tolerance_percent": <positive>, "construction": {...}}');
  }
  const construction = json["construction"];
  if (
    !hasExactly(construction, ["runs", "margin", "measured_on", "measured_max_ms"]) ||
    !isCount(construction["runs"]) ||
    construction["runs"] === 0 ||
    !isPositive(construction["margin"]) ||
    typeof construction["measured_on"] !== "string"
  ) {
    throw new BenchmarkFileError('expected "construction": {"runs": <whole, positive>, "margin": <positive>, "measured_on": <text>, "measured_max_ms": {...}}');
  }
  const measured = construction["measured_max_ms"];
  if (!hasExactly(measured, BROWSERS)) {
    throw new BenchmarkFileError(`expected "measured_max_ms" with exactly ${BROWSERS.join(", ")}`);
  }
  const read = (browser: Browser): number => {
    const value = measured[browser];
    if (!isPositive(value)) {
      throw new BenchmarkFileError(`"measured_max_ms"."${browser}" is not a positive number`);
    }
    return value;
  };
  return {
    tolerancePercent: json["tolerance_percent"],
    construction: {
      runs: construction["runs"],
      margin: construction["margin"],
      measuredOn: construction["measured_on"],
      measuredMaxMs: { chromium: read("chromium"), firefox: read("firefox"), webkit: read("webkit") },
    },
  };
}

/** The bound of the construction of the engine processor in `browser`, in whole milliseconds. */
export function constructionBoundMs(thresholds: Thresholds, browser: Browser): number {
  return Math.ceil(thresholds.construction.measuredMaxMs[browser] * thresholds.construction.margin);
}
