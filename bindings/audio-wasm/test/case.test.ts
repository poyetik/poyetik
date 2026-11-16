import { describe, expect, test } from "vitest";
import { readCase } from "../e2e/site/case.ts";
import { DEFAULT_LIMITER, DETECTORS } from "../ts/engine.ts";

/** What the page passes: the engine's default limiter and its detectors. */
const ENGINE = { limiter: DEFAULT_LIMITER, detectors: DETECTORS };

/** A case of the corpus's shape, every field given. */
const PROTECTED = { sample_rate: 44_100, frames: 300, output: { mode: "protected", lookahead: 2.5, detector: "mastering" } };

describe("the bench page's reading of a case file", () => {
  test("reads a case as the oracle and audio-cli do: raw without an output, limited with one, its lookahead its maximum too", () => {
    expect(readCase({ sample_rate: 8000, frames: 1 }, ENGINE)).toStrictEqual({ rate: 8000, frames: 1, output: { mode: "raw" } });
    expect(readCase({ ...PROTECTED, output: { ...PROTECTED.output, mode: "raw" } }, ENGINE)).toStrictEqual({ rate: 44_100, frames: 300, output: { mode: "raw" } });
    expect(readCase(PROTECTED, ENGINE)).toStrictEqual({
      rate: 44_100,
      frames: 300,
      output: { mode: "protected", maximumLookahead: 2.5, detector: "mastering", limiter: { ceiling: -1, lookahead: 2.5, release: 100, adaptivity: 0.5 } },
    });
  });

  test.each<readonly [unknown, string]>([
    [null, "a case is not an object"],
    [[], "a case is not an object"],
    [{ frames: 1 }, "a case's sample_rate is not a number"],
    [{ sample_rate: "8000", frames: 1 }, "a case's sample_rate is not a number"],
    [{ sample_rate: 8000 }, "a case's frames is not a number"],
    [{ ...PROTECTED, extra: 1 }, "a case holds a field other than sample_rate, frames and output: extra"],
    // A present output is read, never taken for an absent one.
    [{ ...PROTECTED, output: null }, "a case's output is not an object"],
    [{ ...PROTECTED, output: "raw" }, "a case's output is not an object"],
    [{ ...PROTECTED, output: [] }, "a case's output is not an object"],
    [{ ...PROTECTED, output: { ...PROTECTED.output, extra: 1 } }, "a case's output holds a field other than mode, lookahead and detector: extra"],
    [{ ...PROTECTED, output: { lookahead: 1, detector: "precise" } }, "a case's output mode is not one of raw, protected"],
    [{ ...PROTECTED, output: { ...PROTECTED.output, mode: "switchable" } }, "a case's output mode is not one of raw, protected"],
    [{ ...PROTECTED, output: { ...PROTECTED.output, lookahead: "1" } }, "a case's output lookahead is not a number"],
    [{ ...PROTECTED, output: { mode: "raw", detector: "precise" } }, "a case's output lookahead is not a number"],
    [{ ...PROTECTED, output: { mode: "raw", lookahead: 1 } }, "a case's output detector is not one of standard, precise, mastering"],
    [{ ...PROTECTED, output: { ...PROTECTED.output, detector: "exact" } }, "a case's output detector is not one of standard, precise, mastering"],
  ])("refuses %o", (json, reason) => {
    expect(() => readCase(json, ENGINE)).toThrow(new Error(`invalid case: ${reason}`));
  });
});
