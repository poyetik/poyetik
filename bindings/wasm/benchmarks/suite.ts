// The benchmarks of the engine, counted in WebAssembly instructions (spec/benchmarks.md):
// what each measures, the baseline they are compared with, the tolerance, and the law that
// building a plan grows as n log n. Pure functions over a counted module, tested.
import { PRECISIONS, givenAsBinary32, type Precision } from "../ts/engine.ts";
import { CountedInstance, CountError, type CountedEngine } from "./counter.ts";

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

/**
 * The cost of the plan's own sums at one setting: the instructions of {@link BLOCKS} blocks
 * of a patch summing several terms into one place, minus those of the same boxes summing
 * one. So it counts what the plan does per block to add the terms up (the reduction order of
 * spec/engine-design.md, decision 28), which no box benchmark measures.
 */
export interface MixBenchmark {
  /** The setting, in words: what is summed, and the rate. */
  readonly setting: string;
  /** The sample rate, in hertz. */
  readonly rate: number;
  /** A patch summing several terms. */
  readonly patch: Patch;
  /** The same boxes, summing one term where the patch sums several. */
  readonly without: Patch;
}

/**
 * A patch of `oscillators` oscillators (ids 1 to `oscillators`, at different frequencies)
 * whose first `connected` feed the left inlet of one `dac~` (id 0).
 */
export function mixPatch(oscillators: number, connected: number): Patch {
  return {
    format: 1,
    nodes: [{ id: 0, box: "dac~" }, ...Array.from({ length: oscillators }, (_entry, index) => ({ id: index + 1, box: "osc~", params: { frequency: 100 + 37 * (index + 1) } }))],
    connections: Array.from({ length: connected }, (_entry, index) => ({ from: [index + 1, 0] as const, to: [0, 0] as const })),
  };
}

/** The mixing benchmark of `cables` cables into one inlet, minus one cable there. */
function cablesIntoOneInlet(cables: number): MixBenchmark {
  return { setting: `${String(cables)} cables into one inlet at 48 kHz`, rate: 48_000, patch: mixPatch(cables, cables), without: mixPatch(cables, 1) };
}

/**
 * Every mixing benchmark: cables into one inlet in each regime of the reduction order (2:
 * one addition; 3: a partial tree; 16: the full tree; 17: a lane of two terms), and two
 * output boxes on the same channels (whose difference also holds the second box's own
 * work, which `dac~`'s box benchmark measures).
 */
export const MIX_BENCHMARKS: readonly MixBenchmark[] = [
  cablesIntoOneInlet(2),
  cablesIntoOneInlet(3),
  cablesIntoOneInlet(16),
  cablesIntoOneInlet(17),
  {
    setting: "two output boxes fed by one oscillator at 48 kHz",
    rate: 48_000,
    patch: {
      format: 1,
      nodes: [
        { id: 1, box: "osc~", params: { frequency: 440 } },
        { id: 2, box: "dac~" },
        { id: 3, box: "dac~" },
      ],
      connections: [
        { from: [1, 0], to: [2, 0] },
        { from: [1, 0], to: [3, 0] },
      ],
    },
    without: {
      format: 1,
      nodes: [
        { id: 1, box: "osc~", params: { frequency: 440 } },
        { id: 2, box: "dac~" },
      ],
      connections: [{ from: [1, 0], to: [2, 0] }],
    },
  },
];

/**
 * The precision the plans are built in: the default, given as binary32 as in the
 * `AudioWorklet`. Building a plan does the same work in both precisions but for the size of
 * its buffers.
 */
export const PLAN_PRECISION: Precision = "binary64";

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
  /** Each box benchmark in each precision: box, precision, setting and instructions. */
  readonly boxes: readonly { readonly box: string; readonly precision: Precision; readonly setting: string; readonly instructions: number }[];
  /** Each mixing benchmark in each precision: precision, setting and instructions. */
  readonly mixes: readonly { readonly precision: Precision; readonly setting: string; readonly instructions: number }[];
  /** Each plan built: its size in oscillators, and the instructions of loading it. */
  readonly plans: readonly { readonly oscillators: number; readonly instructions: number }[];
  /** The size of the engine module, in bytes: what a page downloads and compiles before it plays. */
  readonly moduleBytes: number;
}

/**
 * A count as a number, which JSON can hold exactly.
 * @throws {RangeError} when it is beyond 2^53 (or below -2^53, which no count reaches).
 */
export function exact(count: bigint): number {
  const value = Number(count);
  if (!Number.isSafeInteger(value)) {
    throw new RangeError(`a count of ${String(count)} instructions cannot be stored exactly`);
  }
  return value;
}

/**
 * Runs `work`, the measurement of the benchmark `name`, so that a failure names it.
 * @throws {CountError} naming the benchmark, with the words of what `work` threw (kept as
 * its cause).
 */
export function forBenchmark<T>(name: string, work: () => T): T {
  try {
    return work();
  } catch (error) {
    throw new CountError(`${name}: ${error instanceof Error ? error.message : String(error)}`, { cause: error });
  }
}

/**
 * The instructions of {@link BLOCKS} blocks of `patch` computed in `precision` and given as
 * binary32, as the `AudioWorklet` renders it, in a fresh instance.
 */
function blocksOf(module: WebAssembly.Module, patch: Patch, rate: number, precision: Precision): bigint {
  const instance = new CountedInstance(module);
  instance.load(patchBytes(patch), rate, givenAsBinary32(precision));
  return instance.process(BLOCKS);
}

/**
 * The instructions of `patch` minus those of `without`, both at `rate` in `precision`: the
 * cost of the benchmark `name`.
 * @throws {CountError} naming it when the engine refuses a patch.
 * @throws {RangeError} naming it when the patch without it costs more than the patch holding it.
 */
function difference(module: WebAssembly.Module, benchmark: { readonly patch: Patch; readonly without: Patch; readonly rate: number }, precision: Precision, name: string): number {
  const [holding, without] = forBenchmark(name, () => [blocksOf(module, benchmark.patch, benchmark.rate, precision), blocksOf(module, benchmark.without, benchmark.rate, precision)]);
  const cost = holding - without;
  if (cost < 0n) {
    throw new RangeError(`${name}: the patch without it costs ${String(-cost)} instructions more than the patch holding it`);
  }
  return exact(cost);
}

/**
 * Where `names` first repeats a name, -1 when it does not: the comparison with the baseline
 * is by name, so a second benchmark of one name would hide the first.
 */
function firstRepeat(names: readonly string[]): number {
  return names.findIndex((name, index) => names.indexOf(name) !== index);
}

/**
 * Runs the benchmarks on a counted engine module: `boxes` then `mixes` in every precision
 * (each benchmark in binary64 then in binary32), then plans of `sizes`, and takes the size
 * of the module.
 * @throws {RangeError} when two benchmarks have one name (box and setting, mixing setting,
 * plan size), or when a box or a mix costs less than nothing (the patch without it costs
 * more), naming it.
 * @throws {CountError} naming the benchmark when the engine refuses a patch.
 */
export function measure(engine: CountedEngine, boxes: readonly BoxBenchmark[], mixes: readonly MixBenchmark[], sizes: readonly number[]): Counts {
  const { module } = engine;
  const names = [...boxes.map((benchmark) => `${benchmark.box} ${benchmark.setting}`), ...mixes.map((benchmark) => `mixing ${benchmark.setting}`), ...sizes.map(planName)];
  const repeated = names[firstRepeat(names)];
  if (repeated !== undefined) {
    throw new RangeError(`${repeated} is measured twice`);
  }
  return {
    boxes: boxes.flatMap((benchmark) =>
      PRECISIONS.map((precision) => {
        const entry = { box: benchmark.box, precision, setting: benchmark.setting };
        return { ...entry, instructions: difference(module, benchmark, precision, boxName(entry)) };
      }),
    ),
    mixes: mixes.flatMap((benchmark) =>
      PRECISIONS.map((precision) => {
        const entry = { precision, setting: benchmark.setting };
        return { ...entry, instructions: difference(module, benchmark, precision, mixName(entry)) };
      }),
    ),
    plans: sizes.map((oscillators) => ({
      oscillators,
      instructions: exact(forBenchmark(planName(oscillators), () => new CountedInstance(module).load(patchBytes(planPatch(oscillators)), PLAN_RATE, givenAsBinary32(PLAN_PRECISION)))),
    })),
    moduleBytes: engine.bytes,
  };
}

/** The name of a box benchmark in the reports: `osc~ 440 Hz at 48 kHz in binary32`. */
export function boxName(entry: { readonly box: string; readonly precision: Precision; readonly setting: string }): string {
  return `${entry.box} ${entry.setting} in ${entry.precision}`;
}

/** The name of a mixing benchmark in the reports: `mixing 2 cables into one inlet at 48 kHz in binary32`. */
export function mixName(entry: { readonly precision: Precision; readonly setting: string }): string {
  return `mixing ${entry.setting} in ${entry.precision}`;
}

/** The name of a plan benchmark in the reports: `loading a plan of 256 oscillators`. */
export function planName(oscillators: number): string {
  return `loading a plan of ${String(oscillators)} oscillators`;
}

/** Every benchmark of `counts`, by name, with its instructions. */
function named(counts: Counts): Map<string, number> {
  return new Map([
    ...counts.boxes.map((entry): [string, number] => [boxName(entry), entry.instructions]),
    ...counts.mixes.map((entry): [string, number] => [mixName(entry), entry.instructions]),
    ...counts.plans.map((entry): [string, number] => [planName(entry.oscillators), entry.instructions]),
  ]);
}

/** A change in percent with two decimals, signed when it is not zero: `+2.10`, `-0.50`, `0.00`. */
export function signedPercent(change: number): string {
  return `${change > 0 ? "+" : ""}${change.toFixed(2)}`;
}

/**
 * The problem of `name` measuring `value` `unit` against `reference`, beyond `tolerance`
 * percent of it either way; none within it.
 */
function moved(name: string, value: number, reference: number, unit: string, tolerance: number): string[] {
  if (Math.abs(value - reference) * 100 <= reference * tolerance) {
    return [];
  }
  const change = signedPercent(((value - reference) * 100) / reference);
  return [`${name}: ${String(value)} ${unit}, baseline ${String(reference)} (${change}%, beyond ±${String(tolerance)}%)`];
}

/**
 * What differs between `measured` and `baseline` beyond `tolerance` percent of the
 * baseline, in either direction (a cost that fell is a baseline to bless, so that it
 * stays tight), the size of the engine module included, and every benchmark found in only
 * one of them. Empty when they agree.
 */
export function regressions(measured: Counts, baseline: Counts, tolerance: number): string[] {
  const now = named(measured);
  const before = named(baseline);
  const problems: string[] = [];
  for (const [name, instructions] of now) {
    const reference = before.get(name);
    if (reference === undefined) {
      problems.push(`${name}: ${String(instructions)} instructions, not in the baseline`);
    } else {
      problems.push(...moved(name, instructions, reference, "instructions", tolerance));
    }
  }
  for (const name of before.keys()) {
    if (!now.has(name)) {
      problems.push(`${name}: in the baseline, measured by no benchmark`);
    }
  }
  return [...problems, ...moved(MODULE_NAME, measured.moduleBytes, baseline.moduleBytes, "bytes", tolerance)];
}

/** The name of the engine module's size in the reports. */
export const MODULE_NAME = "engine module";

/**
 * Where the cost of building plans grows faster than n log n: for each plan and the next
 * larger one, the ratio of their costs must not exceed the ratio of n log2 n. A cost
 * a·n log n + b·n + c with non-negative terms always passes; a quadratic term fails as
 * soon as it dominates. The law holds only for plans of 2 oscillators or more (n log n is 0
 * at 1), each costing something, in increasing size: a plan outside that is a problem too,
 * never a vacuous pass.
 */
export function nLogNProblems(plans: Counts["plans"]): string[] {
  const problems: string[] = [];
  const growth = (size: number): number => size * Math.log2(size);
  let smaller: Counts["plans"][number] | undefined;
  for (const larger of plans) {
    if (larger.oscillators < 2 || larger.instructions === 0) {
      problems.push(`the n log n law needs plans of 2 oscillators or more that cost something: ${planName(larger.oscillators)} costs ${String(larger.instructions)} instructions`);
    } else if (smaller !== undefined && larger.oscillators <= smaller.oscillators) {
      problems.push(`the n log n law needs plans in increasing size: ${String(larger.oscillators)} oscillators after ${String(smaller.oscillators)}`);
    } else if (smaller !== undefined) {
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

/**
 * Whether `value` is a JSON object with exactly the own keys `keys`, in any order: any other
 * key is refused, `__proto__` included (which `JSON.parse` makes an own key). A list has
 * only indices, never `keys`, which are names.
 */
function hasExactly(value: unknown, keys: readonly string[]): value is Record<string, unknown> {
  return value instanceof Object && Object.keys(value).sort().join() === [...keys].sort().join();
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
  if (!hasExactly(json, ["boxes", "mixes", "plans", "module_bytes"]) || !Array.isArray(json["boxes"]) || !Array.isArray(json["mixes"]) || !Array.isArray(json["plans"]) || !isCount(json["module_bytes"])) {
    throw new BenchmarkFileError('expected {"boxes": [...], "mixes": [...], "plans": [...], "module_bytes": <whole>}');
  }
  const moduleBytes = json["module_bytes"];
  const boxes = json["boxes"].map((entry: unknown, index) => {
    if (!hasExactly(entry, ["box", "precision", "setting", "instructions"]) || typeof entry["box"] !== "string" || typeof entry["setting"] !== "string" || !isCount(entry["instructions"])) {
      throw new BenchmarkFileError(`boxes[${String(index)}] is not {"box", "precision", "setting", "instructions"} with a whole count`);
    }
    return { box: entry["box"], precision: precisionOf(entry["precision"], `boxes[${String(index)}]`), setting: entry["setting"], instructions: entry["instructions"] };
  });
  const mixes = json["mixes"].map((entry: unknown, index) => {
    if (!hasExactly(entry, ["precision", "setting", "instructions"]) || typeof entry["setting"] !== "string" || !isCount(entry["instructions"])) {
      throw new BenchmarkFileError(`mixes[${String(index)}] is not {"precision", "setting", "instructions"} with a whole count`);
    }
    return { precision: precisionOf(entry["precision"], `mixes[${String(index)}]`), setting: entry["setting"], instructions: entry["instructions"] };
  });
  const plans = json["plans"].map((entry: unknown, index) => {
    if (!hasExactly(entry, ["oscillators", "instructions"]) || !isCount(entry["oscillators"]) || !isCount(entry["instructions"])) {
      throw new BenchmarkFileError(`plans[${String(index)}] is not {"oscillators", "instructions"} with whole numbers`);
    }
    return { oscillators: entry["oscillators"], instructions: entry["instructions"] };
  });
  for (const [section, names] of [
    ["boxes", boxes.map(boxName)],
    ["mixes", mixes.map(mixName)],
    ["plans", plans.map((entry) => planName(entry.oscillators))],
  ] as const) {
    const index = firstRepeat(names);
    const repeated = names[index];
    if (repeated !== undefined) {
      throw new BenchmarkFileError(`${section}[${String(index)}] repeats "${repeated}"`);
    }
  }
  return { boxes, mixes, plans, moduleBytes };
}

/**
 * The precision `written` names, for the entry at `where`.
 * @throws {BenchmarkFileError} when it names none.
 */
function precisionOf(written: unknown, where: string): Precision {
  const precision = PRECISIONS.find((name) => name === written);
  if (precision === undefined) {
    throw new BenchmarkFileError(`${where}.precision is not one of ${PRECISIONS.join(", ")}`);
  }
  return precision;
}

/** Writes a baseline as its file holds it: two-space indents, a final line break. */
export function formatBaseline(counts: Counts): string {
  return `${JSON.stringify({ boxes: counts.boxes, mixes: counts.mixes, plans: counts.plans, module_bytes: counts.moduleBytes }, null, 2)}\n`;
}

/** The browser engines the construction of the engine processor is timed on, as Playwright names them. */
export const BROWSERS = ["chromium", "firefox", "webkit"] as const;

/** A browser engine of {@link BROWSERS}. */
export type Browser = (typeof BROWSERS)[number];

/** How a wall time is bounded in each browser engine: measured, then multiplied by a margin. */
export interface Timing {
  /** Times taken per browser engine and run. */
  readonly runs: number;
  /** The factor between the slowest time measured and the bound. */
  readonly margin: number;
  /** Where and when the measured values were taken. */
  readonly measuredOn: string;
  /** The slowest of the times measured, per browser engine, in milliseconds. */
  readonly measuredMaxMs: Readonly<Record<Browser, number>>;
}

/** The thresholds of the benchmarks (spec/benchmark-thresholds.json), derived in spec/benchmarks.md. */
export interface Thresholds {
  /** How far, in percent of the baseline, an instruction count may move either way: positive, below 100. */
  readonly tolerancePercent: number;
  /** How the construction of the engine processor is bounded. */
  readonly construction: Timing;
  /**
   * How the wall time of rendering each box benchmark in each precision is bounded: the
   * time of `blocks` blocks of the patch holding the box, every time.
   */
  readonly boxes: Timing & {
    /** Blocks rendered per time taken. */
    readonly blocks: number;
  };
}

/** Whether `value` is a positive, finite number. */
function isPositive(value: unknown): value is number {
  return Number.isFinite(value) && Number(value) > 0;
}

/**
 * Reads a timing of the thresholds file: `keys` exactly, `runs` (whole, positive), `margin`
 * (positive), `measured_on` (text) and `measured_max_ms` (positive, one per browser engine).
 * @throws {BenchmarkFileError} with `shape` when the timing is malformed, or naming the
 * browser engine whose time is.
 */
function parseTiming(value: unknown, keys: readonly string[], shape: string): Timing {
  if (!hasExactly(value, keys) || !isCount(value["runs"]) || value["runs"] === 0 || !isPositive(value["margin"]) || typeof value["measured_on"] !== "string") {
    throw new BenchmarkFileError(shape);
  }
  const measured = value["measured_max_ms"];
  if (!hasExactly(measured, BROWSERS)) {
    throw new BenchmarkFileError(`expected "measured_max_ms" with exactly ${BROWSERS.join(", ")}`);
  }
  const read = (browser: Browser): number => {
    const time = measured[browser];
    if (!isPositive(time)) {
      throw new BenchmarkFileError(`"measured_max_ms"."${browser}" is not a positive number`);
    }
    return time;
  };
  return {
    runs: value["runs"],
    margin: value["margin"],
    measuredOn: value["measured_on"],
    measuredMaxMs: { chromium: read("chromium"), firefox: read("firefox"), webkit: read("webkit") },
  };
}

/** The keys of a timing of the thresholds file. */
const TIMING = ["runs", "margin", "measured_on", "measured_max_ms"] as const;

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
  // Below 100: a count cannot fall by 100% or more, so a wider tolerance could never refuse
  // a fall (spec/benchmarks.md, "Derivation").
  if (!hasExactly(json, ["tolerance_percent", "construction", "boxes"]) || !isPositive(json["tolerance_percent"]) || json["tolerance_percent"] >= 100) {
    throw new BenchmarkFileError('expected {"tolerance_percent": <positive, below 100>, "construction": {...}, "boxes": {...}}');
  }
  const construction = parseTiming(
    json["construction"],
    TIMING,
    'expected "construction": {"runs": <whole, positive>, "margin": <positive>, "measured_on": <text>, "measured_max_ms": {...}}',
  );
  const boxesShape = 'expected "boxes": {"blocks": <whole, positive>, "runs": <whole, positive>, "margin": <positive>, "measured_on": <text>, "measured_max_ms": {...}}';
  const boxes = json["boxes"];
  const timing = parseTiming(boxes, ["blocks", ...TIMING], boxesShape);
  const blocks: unknown = Reflect.get(Object(boxes), "blocks");
  if (!isCount(blocks) || blocks === 0) {
    throw new BenchmarkFileError(boxesShape);
  }
  return { tolerancePercent: json["tolerance_percent"], construction, boxes: { ...timing, blocks } };
}

/** The bound of a timing in `browser`, in whole milliseconds: the slowest time measured times the margin, rounded up. */
export function timingBoundMs(timing: Timing, browser: Browser): number {
  return Math.ceil(timing.measuredMaxMs[browser] * timing.margin);
}

/** The bound of the construction of the engine processor in `browser`, in whole milliseconds. */
export function constructionBoundMs(thresholds: Thresholds, browser: Browser): number {
  return timingBoundMs(thresholds.construction, browser);
}

/** The bound of rendering one box benchmark in one precision in `browser`, in whole milliseconds. */
export function boxBoundMs(thresholds: Thresholds, browser: Browser): number {
  return timingBoundMs(thresholds.boxes, browser);
}
