// The cost benchmarks of a WebAssembly module, whatever the module (spec/benchmarks.md):
// named counts measured on the module as built, compared with a baseline kept in the
// repository within a tolerance in either direction, laws the counts must obey, and the one
// command that checks them or rewrites the baseline. Each library gives its own
// measurements and laws (`Benchmarks`); the comparison and the command are written once,
// here.
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
  /** The command that builds the module, for the error naming a missing one. */
  readonly buildCommand: string;
  /** The command's own command line, for its usage and errors. */
  readonly command: string;
  /** The tolerance, in percent of the baseline, beyond which a count has moved. */
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

/** `change`, a percentage, with its sign and two decimals ("+" unless it is negative). */
function signedPercent(change: number): string {
  const fixed = change.toFixed(2);
  return fixed.startsWith("-") ? fixed : `+${fixed}`;
}

/**
 * What differs between `measured` and `baseline` beyond `tolerance` percent of the
 * baseline, in either direction (a cost that fell is a baseline to bless, so that it stays
 * tight), and every benchmark found in only one of them. Empty when they agree.
 */
export function regressions(measured: readonly Measurement[], baseline: ReadonlyMap<string, number>, tolerance: number): string[] {
  const problems: string[] = [];
  for (const { name, value, unit } of measured) {
    const reference = baseline.get(name);
    if (reference === undefined) {
      problems.push(`${name}: ${String(value)} ${unit}, not in the baseline`);
    } else if (Math.abs(value - reference) * 100 > reference * tolerance) {
      const change = signedPercent(((value - reference) * 100) / reference);
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

/**
 * Reads a baseline: a JSON object of whole counts above 0 by benchmark name, only
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
    const whole = Number(count);
    // A count of 0 is no measurement, and a change from it would be infinite.
    if (!Number.isSafeInteger(count) || whole < 1) {
      throw new Error(`${name}: not a whole count above 0`);
    }
    counts.set(name, whole);
  }
  const written = formatBaseline([...counts].map(([name, count]) => ({ name, value: count })));
  if (written !== text) {
    throw new Error("not as bless writes it (a name written twice, or another layout): restore it from git");
  }
  return counts;
}

/**
 * Fails the command with one line: `what`, then the words of `cause`, which keep what the
 * callee said. The line is all the command reports, so the error keeps no cause.
 */
function failed(what: string, cause: unknown): never {
  throw new Error(`${what}: ${words(cause)}`);
}

/** Measures `benchmarks` on their built module, naming the module when it cannot. */
function measuredOn(io: BenchIo, benchmarks: Benchmarks): readonly Measurement[] {
  let wasm: Uint8Array;
  try {
    wasm = io.read(`${io.repository}/${benchmarks.moduleFile}`);
  } catch (error) {
    failed(`cannot read ${benchmarks.moduleFile} (build it: ${benchmarks.buildCommand})`, error);
  }
  let measurements: readonly Measurement[];
  try {
    measurements = benchmarks.measure(wasm);
  } catch (error) {
    failed(benchmarks.moduleFile, error);
  }
  // Each a whole count, once: a duplicate name would collapse in the baseline, and a value
  // that is not a safe whole number would be written as one the next check refuses.
  const names = new Set<string>();
  for (const { name, value } of measurements) {
    if (names.has(name)) {
      throw new Error(`${benchmarks.moduleFile}: the benchmark ${name} is measured twice`);
    }
    names.add(name);
    if (!Number.isSafeInteger(value) || value < 0) {
      throw new Error(`${benchmarks.moduleFile}: ${name} measured ${String(value)}, not a whole count from 0 to 2^53 - 1`);
    }
  }
  return measurements;
}

/**
 * Refuses a tolerance that is not a finite percentage of 0 or more: a NaN would pass every
 * count, a negative one fail every count.
 * @throws {Error} naming the module and the tolerance.
 */
function checkedTolerance(benchmarks: Benchmarks): void {
  const tolerance = benchmarks.tolerancePercent;
  if (!Number.isFinite(tolerance) || tolerance < 0) {
    throw new Error(`the tolerance of ${benchmarks.moduleFile}'s benchmarks, ${String(tolerance)}%, is not a finite percentage of 0 or more`);
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

/** Reads the baseline, naming its file when it cannot. */
function baselineOf(io: BenchIo, benchmarks: Benchmarks): Map<string, number> {
  let text: string;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(io.read(`${io.repository}/${benchmarks.baselineFile}`));
  } catch (error) {
    failed(`cannot read ${benchmarks.baselineFile}`, error);
  }
  try {
    return parseBaseline(text);
  } catch (error) {
    failed(benchmarks.baselineFile, error);
  }
}

/** `check`: compares the measurements with the baseline and the laws. */
function check(io: BenchIo, benchmarks: Benchmarks): number {
  const baseline = baselineOf(io, benchmarks);
  const measurements = measuredOn(io, benchmarks);
  report(io, measurements);
  return verdict(
    io,
    [...regressions(measurements, baseline, benchmarks.tolerancePercent), ...benchmarks.laws(measurements)],
    `the benchmarks of ${benchmarks.moduleFile} moved beyond ±${String(benchmarks.tolerancePercent)}% of ${benchmarks.baselineFile}, or break their laws`,
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
    io.err(`error: ${command === undefined ? "no command given" : oneLine(`unknown command "${command}"`)} (see: ${benchmarks.command} help)`);
    return 2;
  }
  if (rest.length > 0) {
    io.err(`error: \`${command}\` takes no argument (see: ${benchmarks.command} help)`);
    return 2;
  }
  try {
    switch (command) {
      case "help":
        io.out(`usage: ${benchmarks.command} <check|bless|help>\n  check  measure ${benchmarks.moduleFile} and compare it with ${benchmarks.baselineFile}, within ±${String(benchmarks.tolerancePercent)}%, and with its laws\n  bless  measure it and rewrite ${benchmarks.baselineFile}, if its laws hold\n  help   print this help`);
        return 0;
      case "check":
        checkedTolerance(benchmarks);
        return check(io, benchmarks);
      case "bless":
        checkedTolerance(benchmarks);
        return bless(io, benchmarks);
    }
  } catch (error) {
    io.err(`error: ${oneLine(words(error))}`);
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
