// The cost benchmarks of a WebAssembly module, whatever the module (spec/benchmarks.md):
// named counts measured on the module as built, compared with a baseline kept in the
// repository within a tolerance in either direction, laws the counts must obey, and the one
// command that checks them or rewrites the baseline. Each library gives its own
// measurements and laws (`Benchmarks`); the comparison and the command are written once,
// here, for the visual library's benchmarks so far: the engine's are to come here at roadmap
// #24 step 165.
import { oneLine } from "@poyetik/config/one-line";
import { COUNTER } from "./instrument.ts";

/** One named count: what a benchmark measured, in its unit. */
export interface Measurement {
  /** The benchmark's name, unique in its suite, as the reports and the baseline write it. */
  readonly name: string;
  /** What it measured: a whole number. */
  readonly value: number;
  /** The unit of the value, plural (`instructions`, `bytes`). */
  readonly unit: string;
}

/** What a library measures, where its files are, and what its counts must obey. */
export interface Benchmarks {
  /** The baseline, from the repository root. */
  readonly baselineFile: string;
  /** The built module the measurements read, from the repository root. */
  readonly moduleFile: string;
  /** The package whose scripts build the module (`build:wasm`), check it (`benchmarks:check`) and run the command (`scripts/benchmarks.ts`), as the hints name them, from any directory of the workspace. */
  readonly packageName: string;
  /** The tolerance, a whole percentage of the baseline, beyond which a count has moved. */
  readonly tolerancePercent: number;
  /**
   * Measures the module `wasm`, in the order the reports and the baseline keep.
   * @throws {Error} naming the benchmark that could not be measured.
   */
  readonly measure: (wasm: Uint8Array) => readonly Measurement[];
  /** The laws the measurements must obey, each problem in one line; none when they do. */
  readonly laws: (measurements: readonly Measurement[]) => readonly string[];
}

/** What the command needs from the outside world. */
export interface BenchIo {
  /** The repository root, without a trailing slash. */
  readonly repository: string;
  /** Reads a file as bytes; throws when it cannot. */
  readonly read: (path: string) => Uint8Array;
  /** Writes a text file; throws when it cannot. */
  readonly write: (path: string, text: string) => void;
  /** Writes a line on stdout. */
  readonly out: (line: string) => void;
  /** Writes a line on stderr. */
  readonly err: (line: string) => void;
}

/**
 * `count`, a count of executed instructions, as an exact number.
 * @throws {RangeError} when it is above 2⁵³ − 1, where a number would round it.
 */
export function exact(count: bigint): number {
  const value = Number(count);
  if (!Number.isSafeInteger(value)) {
    throw new RangeError(`a count of ${String(count)} cannot be stored exactly`);
  }
  return value;
}

/**
 * Runs `work`, the measurement of the benchmark `name`, so that a failure names it.
 * @throws {Error} naming the benchmark, with the words of what `work` threw (kept as its
 * cause).
 */
export function forBenchmark<T>(name: string, work: () => T): T {
  try {
    return work();
  } catch (error) {
    throw new Error(`${name}: ${words(error)}`, { cause: error });
  }
}

/** The words of what was thrown: an error's message, or the thrown value as text. */
function words(thrown: unknown): string {
  return thrown instanceof Error ? thrown.message : String(thrown);
}

/**
 * The change from `reference` by `difference`, a percentage with its sign ("+" unless it is
 * negative) and two decimals, rounded away from zero, so that a change beyond a tolerance
 * never reads as within it.
 */
function signedPercent(difference: number, reference: number): string {
  const base = BigInt(reference);
  const hundredths = (BigInt(Math.abs(difference)) * 10_000n + base - 1n) / base;
  return `${String(difference).startsWith("-") ? "-" : "+"}${String(hundredths / 100n)}.${String(hundredths % 100n).padStart(2, "0")}`;
}

/**
 * What differs between `measured` and `baseline` beyond `tolerance` percent of the
 * baseline, in either direction (a cost that fell is a baseline to bless, so that it stays
 * tight), and every benchmark found in only one of them. Empty when they agree. Compared
 * exactly, in integers: the counts and the tolerance are whole ({@link isCount},
 * `checkedTolerance`), and their products may pass 2⁵³, where a number would round them.
 * @throws {RangeError} When a count or the tolerance is not an integer, or a baseline's count
 *   is 0 where a measured one differs from it; a count of 0 or below otherwise is compared
 *   without throwing, its verdict meaningless. The command gives it none of these:
 *   {@link isCount} keeps them out at parsing.
 */
export function regressions(measured: readonly Measurement[], baseline: ReadonlyMap<string, number>, tolerance: number): string[] {
  const problems: string[] = [];
  for (const { name, value, unit } of measured) {
    const reference = baseline.get(name);
    if (reference === undefined) {
      problems.push(`${name}: ${String(value)} ${unit}, not in the baseline`);
    } else if (BigInt(Math.abs(value - reference)) * 100n > BigInt(reference) * BigInt(tolerance)) {
      const change = signedPercent(value - reference, reference);
      problems.push(`${name}: ${String(value)} ${unit}, baseline ${String(reference)} (${change}%, beyond ±${String(tolerance)}%)`);
    }
  }
  const names = new Set(measured.map(({ name }) => name));
  for (const name of baseline.keys()) {
    if (!names.has(name)) {
      problems.push(`${name}: in the baseline, measured by no benchmark`);
    }
  }
  return problems;
}

/** The baseline of `measurements`, exactly as `bless` writes it: one name and value a line, in their order. */
export function formatBaseline(measurements: readonly Pick<Measurement, "name" | "value">[]): string {
  return `${JSON.stringify(Object.fromEntries(measurements.map(({ name, value }) => [name, value])), undefined, 2)}\n`;
}

/** What a count of a baseline is, as both {@link isCount}'s callers say it. */
const COUNT = "a whole count from 1 to 2^53 - 1";

/**
 * Whether `value` is a count a baseline holds: a whole number from 1 to 2⁵³ − 1, exact in
 * JSON. The one rule of what `bless` writes (each measurement) and of what the reader of
 * the baseline accepts ({@link parseBaseline}), so that `bless` never writes a baseline the
 * next check refuses. A count of 0 is no measurement, and a change from it would be
 * infinite.
 * @param value The value.
 * @returns Whether it is such a count.
 */
export function isCount(value: unknown): value is number {
  // Each clause decides a case alone (no predicate after the first refuses a non-number by itself:
  // a string such as "1" passes the comparisons and the remainder), so that none is redundant.
  return typeof value === "number" && value >= 1 && value <= Number.MAX_SAFE_INTEGER && value % 1 === 0;
}

/**
 * Reads a baseline: a JSON object of counts ({@link isCount}) by benchmark name, only
 * exactly as {@link formatBaseline} writes it.
 * @throws {Error} saying what is wrong with it.
 */
export function parseBaseline(text: string): Map<string, number> {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch (error) {
    throw new Error(`not JSON (${words(error)})`, { cause: error });
  }
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("not a JSON object of counts by benchmark name");
  }
  const counts = new Map<string, number>();
  for (const [name, count] of Object.entries(value)) {
    if (!isCount(count)) {
      throw new Error(`${name}: not ${COUNT}`);
    }
    counts.set(name, count);
  }
  const written = formatBaseline([...counts].map(([name, count]) => ({ name, value: count })));
  if (written !== text) {
    throw new Error("not as bless writes it (a name written twice, or another layout)");
  }
  return counts;
}

/** A failure of the command: its reason, which the line bounds, and its remedy, which follows whole. */
class BenchFailure extends Error {
  /** What helps (`: <remedy>`), or nothing. */
  readonly remedy: string;

  /** A failure for `reason`, which `remedy` puts right. */
  constructor(reason: string, remedy: string) {
    super(reason);
    this.remedy = remedy;
  }
}

/**
 * Fails the command with one line: `what`, then the words of `cause`, which keep what the
 * callee said, then `remedy`, which no reason cuts. The line is all the command reports, so
 * the error keeps no cause.
 */
function failed(what: string, cause: unknown, remedy = ""): never {
  throw new BenchFailure(`${what}: ${words(cause)}`, remedy);
}

/** Measures `benchmarks` on their built module, naming the module when it cannot. */
function measuredOn(io: BenchIo, benchmarks: Benchmarks): readonly Measurement[] {
  let wasm: Uint8Array;
  try {
    wasm = io.read(`${io.repository}/${benchmarks.moduleFile}`);
  } catch (error) {
    failed(`cannot read ${benchmarks.moduleFile}`, error, errorCode(error) === "ENOENT" ? `: build it (see: pnpm --filter ${benchmarks.packageName} build:wasm)` : "");
  }
  let measurements: readonly Measurement[];
  try {
    measurements = benchmarks.measure(wasm);
  } catch (error) {
    failed(benchmarks.moduleFile, error);
  }
  // Each a count, once: a duplicate name would collapse in the baseline, and a value that is
  // not a count would be written as one the next check refuses (the same rule, isCount).
  const names = new Set<string>();
  for (const { name, value } of measurements) {
    if (names.has(name)) {
      throw new Error(`${benchmarks.moduleFile}: the benchmark ${name} is measured twice`);
    }
    names.add(name);
    if (!isCount(value)) {
      throw new Error(`${benchmarks.moduleFile}: ${name} measured ${String(value)}, not ${COUNT}`);
    }
  }
  return measurements;
}

/**
 * Refuses a tolerance that is not a whole percentage of 0 or more, exact as a number (up to
 * 2⁵³ − 1): a NaN would pass every count, a negative one fail every count, and a fraction
 * would need a comparison of rounded products ({@link regressions} compares in integers).
 * @throws {Error} naming the module and the tolerance.
 */
function checkedTolerance(benchmarks: Benchmarks): void {
  const tolerance = benchmarks.tolerancePercent;
  if (!Number.isSafeInteger(tolerance) || tolerance < 0) {
    throw new Error(`the tolerance of ${benchmarks.moduleFile}'s benchmarks, ${String(tolerance)}%, is not a whole percentage of 0 or more`);
  }
}

/** Prints every measurement, one per line. */
function report(io: BenchIo, measurements: readonly Measurement[]): void {
  for (const { name, value, unit } of measurements) {
    io.out(`${name}: ${String(value)} ${unit}`);
  }
}

/** Prints `problems` on stdout and one `error:` line about `what`; returns the exit code. */
function verdict(io: BenchIo, problems: readonly string[], what: string): number {
  if (problems.length === 0) {
    return 0;
  }
  for (const problem of problems) {
    io.out(oneLine(problem));
  }
  io.err(`error: ${oneLine(what)} (${String(problems.length)} problem(s); see spec/benchmarks.md)`);
  return 1;
}

/** What `thrown` holds as its code, whatever it is: a Node.js system error's (`ENOENT`, `EISDIR`, `EACCES`), else undefined. */
function errorCode(thrown: unknown): unknown {
  return Reflect.get(Object(thrown), "code");
}

/**
 * The git command restoring the baseline as last committed, never from the index, which may
 * hold the damage staged, a runnable hint (`CLAUDE.md`, "A hint is a command"): its path in
 * git's `:/` form, from the repository's root, so that it runs as written from any
 * directory of the repository, the package's, where the command runs, included.
 */
function restoreHint(benchmarks: Benchmarks): string {
  return `(see: git checkout HEAD -- :/${benchmarks.baselineFile})`;
}

/** What a baseline not as `bless` writes it asks: restored, or rewritten only when the counts are meant to change. */
function restoreOrRewrite(benchmarks: Benchmarks): string {
  return `: restore it ${restoreHint(benchmarks)}, or rewrite it only when the counts are meant to change (see: cargo xtask bless-benchmarks)`;
}

/**
 * What a baseline that cannot be read asks, by the code of its error (`: <remedy>`, or
 * nothing): a missing one restored, or blessed; a directory in its place moved away, or a
 * permission made readable, then the check run again (never a restore beside a directory: git
 * would delete it with the untracked files it holds); any other failure, which no command of
 * the repository fixes, the error alone.
 */
function readRemedy(thrown: unknown, benchmarks: Benchmarks): string {
  const code = errorCode(thrown);
  const again = `then check again (see: pnpm --filter ${benchmarks.packageName} benchmarks:check)`;
  if (code === "ENOENT") {
    return `: restore it ${restoreHint(benchmarks)}, or bless it (see: cargo xtask bless-benchmarks)`;
  }
  if (code === "EISDIR") {
    return `: a directory stands where the file should be: move it away, ${again}`;
  }
  if (code === "EACCES" || code === "EPERM") {
    return `: make it readable, ${again}`;
  }
  return "";
}

/**
 * Reads the baseline, naming its file and what helps by the kind of the failure: a read
 * that fails by its code ({@link readRemedy}); bytes that are not UTF-8 text, or text not as
 * `bless` writes it, restored or rewritten ({@link restoreOrRewrite}).
 */
function baselineOf(io: BenchIo, benchmarks: Benchmarks): Map<string, number> {
  let bytes: Uint8Array;
  try {
    bytes = io.read(`${io.repository}/${benchmarks.baselineFile}`);
  } catch (error) {
    failed(`cannot read ${benchmarks.baselineFile}`, error, readRemedy(error, benchmarks));
  }
  let text: string;
  try {
    text = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes);
  } catch (error) {
    failed(benchmarks.baselineFile, `not UTF-8 text, not as bless writes it (${words(error)})`, restoreOrRewrite(benchmarks));
  }
  try {
    return parseBaseline(text);
  } catch (error) {
    failed(benchmarks.baselineFile, error, restoreOrRewrite(benchmarks));
  }
}

/** The command line that runs the command, from any directory of the workspace. */
function scriptCommand(benchmarks: Benchmarks): string {
  return `pnpm --filter ${benchmarks.packageName} exec node scripts/benchmarks.ts`;
}

/** `check`: compares the measurements with the baseline and the laws. */
function check(io: BenchIo, benchmarks: Benchmarks): number {
  const baseline = baselineOf(io, benchmarks);
  const measurements = measuredOn(io, benchmarks);
  report(io, measurements);
  const broken = benchmarks.laws(measurements);
  const moved = `the benchmarks of ${benchmarks.moduleFile} moved beyond ±${String(benchmarks.tolerancePercent)}% of ${benchmarks.baselineFile}`;
  return verdict(
    io,
    [...regressions(measurements, baseline, benchmarks.tolerancePercent), ...broken],
    // The bless refuses broken laws: named only when it would write.
    broken.length > 0 ? `${moved}, or break their laws` : `${moved}: bless them only when the change is meant (see: cargo xtask bless-benchmarks)`,
  );
}

/** `bless`: rewrites the baseline, unless the measurements break their laws. */
function bless(io: BenchIo, benchmarks: Benchmarks): number {
  const measurements = measuredOn(io, benchmarks);
  report(io, measurements);
  const status = verdict(io, benchmarks.laws(measurements), `no ${benchmarks.baselineFile} written: the benchmarks of ${benchmarks.moduleFile} break their laws`);
  if (status !== 0) {
    return status;
  }
  try {
    io.write(`${io.repository}/${benchmarks.baselineFile}`, formatBaseline(measurements));
  } catch (error) {
    failed(`cannot write ${benchmarks.baselineFile}`, error);
  }
  io.out(`wrote ${benchmarks.baselineFile}`);
  return 0;
}

/**
 * The command: runs `args` (the program name excluded) and returns the exit code, as every
 * command-line tool of the repository does: 0 on success, 1 when the work failed (one
 * `error:` line on stderr, the problems on stdout), 2 for a usage error.
 */
export function benchMain(io: BenchIo, args: readonly string[], benchmarks: Benchmarks): number {
  const [command, ...rest] = args;
  if (command !== "check" && command !== "bless" && command !== "help") {
    io.err(`error: ${command === undefined ? "no command given" : oneLine(`unknown command "${command}"`)} (see: ${scriptCommand(benchmarks)} help)`);
    return 2;
  }
  if (rest.length > 0) {
    io.err(`error: \`${command}\` takes no argument (see: ${scriptCommand(benchmarks)} help)`);
    return 2;
  }
  try {
    switch (command) {
      case "help":
        io.out(`usage: ${scriptCommand(benchmarks)} <check|bless|help>\n  check  measure ${benchmarks.moduleFile} and compare it with ${benchmarks.baselineFile}, within ±${String(benchmarks.tolerancePercent)}%, and with its laws\n  bless  measure it and rewrite ${benchmarks.baselineFile}, if its laws hold, only through cargo xtask bless-benchmarks\n  help   print this help`);
        return 0;
      case "check":
        checkedTolerance(benchmarks);
        return check(io, benchmarks);
      case "bless":
        checkedTolerance(benchmarks);
        return bless(io, benchmarks);
    }
  } catch (error) {
    io.err(`error: ${oneLine(words(error))}${error instanceof BenchFailure ? error.remedy : ""}`);
    return 1;
  }
}

/**
 * Calls the export `name` of `exports` with `args` and returns what it returns: how a
 * library's measurements and tests call a counted instance's exports, read as unknown values.
 * @param exports The instance's exports.
 * @param name The export.
 * @param args Its arguments.
 * @returns What it returned, unchecked.
 * @throws {TypeError} when it is not a function.
 */
export function call(exports: Readonly<Record<string, unknown>>, name: string, ...args: readonly number[]): unknown {
  const exported = exports[name];
  if (typeof exported !== "function") {
    throw new TypeError(`no function ${name}`);
  }
  return Reflect.apply(exported, undefined, args);
}

/**
 * `value`, the counter's value, as an i64 global gives it.
 * @throws {Error} when it is not a bigint: the counter is not an i64.
 */
export function counterValue(value: unknown): bigint {
  if (typeof value !== "bigint") {
    throw new Error(`the counter "${COUNTER}" is not an i64`);
  }
  return value;
}

/** An instance of an instrumented module, and the count of what an action executes in it. */
export interface CountedInstance {
  /** The instance's exports, read at the boundary as unknown values. */
  readonly exports: Readonly<Record<string, unknown>>;
  /** The instructions `action` executes in the instance. */
  readonly counted: (action: () => void) => bigint;
}

/**
 * Instantiates `module`, instrumented by `instrument` (no imports), with its counter.
 * @throws {Error} when the module does not export its counter (and, when counting, when
 * the counter is not an i64).
 */
export function countedInstance(module: WebAssembly.Module): CountedInstance {
  const { exports } = new WebAssembly.Instance(module, {});
  const counter = exports[COUNTER];
  if (!(counter instanceof WebAssembly.Global)) {
    throw new Error(`the module does not export its counter "${COUNTER}"`);
  }
  const instructions = (): bigint => counterValue(counter.value);
  return {
    exports,
    counted: (action) => {
      const before = instructions();
      action();
      return instructions() - before;
    },
  };
}
