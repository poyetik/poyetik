// The benchmark command (scripts/benchmarks.ts): `check` compares the instruction counts of the
// built engine module with the baseline and the n log n law, `bless` rewrites the baseline
// (run only through `cargo xtask bless-benchmarks`, in a commit that says why). Everything
// it touches is injected, so that it is tested; the entry point wires the real process.
import { oneLine } from "@poyetik/config/one-line";
import { countedEngine } from "./counter.ts";
import { BOX_BENCHMARKS, boxName, formatBaseline, measure, MIX_BENCHMARKS, mixName, MODULE_NAME, nLogNProblems, parseBaseline, parseThresholds, planName, PLAN_SIZES, regressions, type BoxBenchmark, type Counts, type MixBenchmark } from "./suite.ts";

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

/** What is measured: the box benchmarks, the mixing benchmarks, and the sizes of the plans built. */
export interface Suite {
  /** The box benchmarks. */
  readonly boxes: readonly BoxBenchmark[];
  /** The mixing benchmarks. */
  readonly mixes: readonly MixBenchmark[];
  /** The plan sizes, in oscillators, increasing. */
  readonly sizes: readonly number[];
}

/** The benchmarks `check` and `bless` run. */
export const SUITE: Suite = { boxes: BOX_BENCHMARKS, mixes: MIX_BENCHMARKS, sizes: PLAN_SIZES };

/** The baseline, from the repository root. */
export const BASELINE_FILE = "spec/benchmark-baseline.json";

/** The thresholds, from the repository root. */
export const THRESHOLDS_FILE = "spec/benchmark-thresholds.json";

/** The engine module measured, from the repository root (`pnpm build` writes it). */
export const MODULE_FILE = "bindings/wasm/dist/engine.wasm";

/** The usage, printed by `help` only. */
export const USAGE = `usage: node scripts/benchmarks.ts <check|bless|help> (in bindings/wasm)
  check   count the instructions of every benchmark on ${MODULE_FILE}, and its size: fails on a count
          beyond the tolerance of ${THRESHOLDS_FILE} from ${BASELINE_FILE},
          either way, on a benchmark missing from either, or on plans growing faster than n log n
  bless   rewrite ${BASELINE_FILE} from the counts (through cargo xtask bless-benchmarks)
  help    show this help`;

/**
 * Fails the command with one line: `what`, then the words of `cause` (which keep what the
 * callee said; the line is all the command reports).
 */
function failed(what: string, cause: unknown): never {
  throw new Error(`${what}: ${words(cause)}`);
}

/** The words of what was thrown: an error's message, or the thrown value as text. */
function words(thrown: unknown): string {
  return thrown instanceof Error ? thrown.message : String(thrown);
}

/** Reads a file of the repository as text, naming it when it cannot. */
function readText(io: BenchIo, file: string): string {
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(io.read(`${io.repository}/${file}`));
  } catch (error) {
    failed(`cannot read ${file}`, error);
  }
}

/** Runs `parse` on the text of `file`, naming the file when it is malformed. */
function parsed<T>(io: BenchIo, file: string, parse: (text: string) => T): T {
  const text = readText(io, file);
  try {
    return parse(text);
  } catch (error) {
    failed(file, error);
  }
}

/** Measures `suite` on the built engine module. */
function measured(io: BenchIo, suite: Suite): Counts {
  let wasm: Uint8Array;
  try {
    wasm = io.read(`${io.repository}/${MODULE_FILE}`);
  } catch (error) {
    failed(`cannot read ${MODULE_FILE}`, error);
  }
  try {
    return measure(countedEngine(wasm), suite.boxes, suite.mixes, suite.sizes);
  } catch (error) {
    failed(MODULE_FILE, error);
  }
}

/** Prints every count, one per line. */
function report(io: BenchIo, counts: Counts): void {
  for (const entry of counts.boxes) {
    io.out(`${boxName(entry)}: ${String(entry.instructions)} instructions`);
  }
  for (const entry of counts.mixes) {
    io.out(`${mixName(entry)}: ${String(entry.instructions)} instructions`);
  }
  for (const entry of counts.plans) {
    io.out(`${planName(entry.oscillators)}: ${String(entry.instructions)} instructions`);
  }
  io.out(`${MODULE_NAME}: ${String(counts.moduleBytes)} bytes`);
}

/** Prints `problems` on stdout and one `error:` line about `what`; returns the exit code. */
function verdict(io: BenchIo, problems: readonly string[], what: string): number {
  if (problems.length === 0) {
    return 0;
  }
  for (const problem of problems) {
    io.out(oneLine(problem));
  }
  io.err(`error: ${what} (${String(problems.length)} problem(s); see spec/benchmarks.md)`);
  return 1;
}

/** `check`: compares the counts with the baseline and the law. */
function check(io: BenchIo, suite: Suite): number {
  const thresholds = parsed(io, THRESHOLDS_FILE, parseThresholds);
  const baseline = parsed(io, BASELINE_FILE, parseBaseline);
  const counts = measured(io, suite);
  report(io, counts);
  return verdict(io, [...regressions(counts, baseline, thresholds.tolerancePercent), ...nLogNProblems(counts.plans)], "the benchmarks moved beyond their thresholds");
}

/** `bless`: rewrites the baseline, unless the plans break the n log n law. */
function bless(io: BenchIo, suite: Suite): number {
  const counts = measured(io, suite);
  report(io, counts);
  const status = verdict(io, nLogNProblems(counts.plans), "no baseline written: building plans breaks its law");
  if (status !== 0) {
    return status;
  }
  try {
    io.write(`${io.repository}/${BASELINE_FILE}`, formatBaseline(counts));
  } catch (error) {
    failed(`cannot write ${BASELINE_FILE}`, error);
  }
  io.out(`wrote ${BASELINE_FILE}`);
  return 0;
}

/**
 * The command: runs `args` (the program name excluded) and returns the exit code, as every
 * command-line tool of the repository does: 0 on success, 1 when the work failed (one
 * `error:` line on stderr, the problems on stdout), 2 for a usage error.
 */
export function benchMain(io: BenchIo, args: readonly string[], suite: Suite = SUITE): number {
  const [command, ...rest] = args;
  if (command !== "check" && command !== "bless" && command !== "help") {
    io.err(`error: ${command === undefined ? "no command given" : oneLine(`unknown command "${command}"`)} (see: node scripts/benchmarks.ts help)`);
    return 2;
  }
  if (rest.length > 0) {
    io.err(`error: \`${command}\` takes no argument (see: node scripts/benchmarks.ts help)`);
    return 2;
  }
  try {
    switch (command) {
      case "help":
        io.out(USAGE);
        return 0;
      case "check":
        return check(io, suite);
      case "bless":
        return bless(io, suite);
    }
  } catch (error) {
    io.err(`error: ${oneLine(words(error))}`);
    return 1;
  }
}
