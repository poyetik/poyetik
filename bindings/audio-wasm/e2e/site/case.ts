// How the bench page (harness.ts) reads a conformance case file: as the oracle and
// audio-cli read it, its fields exactly, a present output never taken for an absent one;
// measured and mutated here, while harness.ts only fetches the file. The engine checks the
// values' ranges, and the conformance readers the rest of the format.
import type { Detector, LimiterSettings, Output } from "../../dist/web.js";

/** A conformance case, as the page renders it. */
export interface Case {
  /** Sample rate of the render, in hertz. */
  readonly rate: number;
  /** Duration of the render, in frames. */
  readonly frames: number;
  /** The output stage of the render: raw when the case names none. */
  readonly output: Output;
}

/** What the page takes from the engine to read a case: its default limiter and its detectors. */
export interface CaseEngine {
  /** The limiter's settings a case does not name (all but the lookahead). */
  readonly limiter: LimiterSettings;
  /** The detectors, by name. */
  readonly detectors: readonly Detector[];
}

/** The modes a case's output stage may name. */
const CASE_MODES = ["raw", "protected"] as const;

/** Refuses a case file, saying why. */
function invalid(reason: string): Error {
  return new Error(`invalid case: ${reason}`);
}

/** `value` as an object holding only the fields `known`, or refused as `what`. */
function fields(value: unknown, known: readonly string[], what: string): object {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw invalid(`${what} is not an object`);
  }
  const other = Object.keys(value).find((key) => !known.includes(key));
  if (other !== undefined) {
    throw invalid(`${what} holds a field other than ${known.slice(0, -1).join(", ")} and ${String(known.at(-1))}: ${other}`);
  }
  return value;
}

/** The number in field `name` of `value`, or refused as `what`'s. */
function number(value: object, name: string, what: string): number {
  const field: unknown = Reflect.get(value, name);
  if (typeof field !== "number") {
    throw invalid(`${what} ${name} is not a number`);
  }
  return field;
}

/**
 * The output stage a case names (`output`: its mode, raw or protected, its limiter's
 * lookahead, its maximum too, and its detector, all three named whatever the mode, as the
 * oracle reads them), as an engine's: a raw one without a limiter, a protected one with the
 * limiter's other settings `engine`'s defaults.
 */
function caseOutput(value: unknown, engine: CaseEngine): Output {
  const output = fields(value, ["mode", "lookahead", "detector"], "a case's output");
  const mode = CASE_MODES.find((known) => known === Reflect.get(output, "mode"));
  if (mode === undefined) {
    throw invalid(`a case's output mode is not one of ${CASE_MODES.join(", ")}`);
  }
  const lookahead = number(output, "lookahead", "a case's output");
  const detector = engine.detectors.find((known) => known === Reflect.get(output, "detector"));
  if (detector === undefined) {
    throw invalid(`a case's output detector is not one of ${engine.detectors.join(", ")}`);
  }
  return mode === "raw" ? { mode } : { mode, maximumLookahead: lookahead, detector, limiter: { ...engine.limiter, lookahead } };
}

/**
 * Reads the parsed JSON of a case file: an object with exactly `sample_rate`, `frames` and,
 * optionally, `output`; raw when it has no `output`, a present one read whatever it holds.
 * @throws {Error} naming what is wrong, when the file is not such a case.
 */
export function readCase(json: unknown, engine: CaseEngine): Case {
  const value = fields(json, ["sample_rate", "frames", "output"], "a case");
  const rate = number(value, "sample_rate", "a case's");
  const frames = number(value, "frames", "a case's");
  return { rate, frames, output: Object.hasOwn(value, "output") ? caseOutput(Reflect.get(value, "output"), engine) : { mode: "raw" } };
}
