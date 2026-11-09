import { readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";
import { CountedInstance, CountError, countedModule } from "../../benchmarks/counter.ts";
import {
  BLOCKS,
  BOX_BENCHMARKS,
  BROWSERS,
  BenchmarkFileError,
  boxBoundMs,
  constructionBoundMs,
  formatBaseline,
  measure,
  nLogNProblems,
  parseBaseline,
  parseThresholds,
  PLAN_PRECISION,
  PLAN_RATE,
  PLAN_SIZES,
  planPatch,
  patchBytes,
  regressions,
  signedPercent,
  type Counts,
} from "../../benchmarks/suite.ts";
import { SPEC, wasmBytes } from "../fixtures.ts";

/** The built engine module, instrumented and compiled at the first use (a mutant that breaks it fails a test, not the file). */
let compiled: WebAssembly.Module | undefined;

/** {@link compiled}, compiled if needed. */
function counted(): WebAssembly.Module {
  compiled ??= countedModule(new Uint8Array(wasmBytes()));
  return compiled;
}

describe("the benchmarks", () => {
  test("measure every box type at its settings, rates included", () => {
    expect(BOX_BENCHMARKS.map(({ box, setting, rate }) => [box, setting, rate])).toStrictEqual([
      ["osc~", "440 Hz at 48 kHz", 48_000],
      ["osc~", "1000 Hz at 8 kHz", 8000],
      ["osc~", "19999 Hz at 384 kHz", 384_000],
      ["osc~", "0 Hz at 44.1 kHz", 44_100],
      ["dac~", "two inlets fed at 48 kHz", 48_000],
      ["dac~", "unconnected at 48 kHz", 48_000],
    ]);
    expect([BLOCKS, PLAN_RATE, PLAN_SIZES, PLAN_PRECISION]).toStrictEqual([64, 48_000, [256, 1024, 4096, 16_384], "binary64"]);
  });

  test("hold each box in its patch, and not in the patch without it", () => {
    const text = (patch: Parameters<typeof patchBytes>[0]): string => new TextDecoder().decode(patchBytes(patch));
    expect(BOX_BENCHMARKS.map((benchmark) => [text(benchmark.patch), text(benchmark.without)])).toStrictEqual([
      ['{"format":1,"nodes":[{"id":1,"box":"osc~","params":{"frequency":440}}],"connections":[]}', '{"format":1,"nodes":[],"connections":[]}'],
      ['{"format":1,"nodes":[{"id":1,"box":"osc~","params":{"frequency":1000}}],"connections":[]}', '{"format":1,"nodes":[],"connections":[]}'],
      ['{"format":1,"nodes":[{"id":1,"box":"osc~","params":{"frequency":19999}}],"connections":[]}', '{"format":1,"nodes":[],"connections":[]}'],
      ['{"format":1,"nodes":[{"id":1,"box":"osc~","params":{"frequency":0}}],"connections":[]}', '{"format":1,"nodes":[],"connections":[]}'],
      [
        '{"format":1,"nodes":[{"id":1,"box":"osc~","params":{"frequency":440}},{"id":2,"box":"dac~"}],"connections":[{"from":[1,0],"to":[2,0]},{"from":[1,0],"to":[2,1]}]}',
        '{"format":1,"nodes":[{"id":1,"box":"osc~","params":{"frequency":440}}],"connections":[]}',
      ],
      ['{"format":1,"nodes":[{"id":1,"box":"dac~"}],"connections":[]}', '{"format":1,"nodes":[],"connections":[]}'],
    ]);
  });

  test("build plans of oscillators feeding both inlets of one output in turn", () => {
    expect(new TextDecoder().decode(patchBytes(planPatch(3)))).toBe(
      '{"format":1,"nodes":[{"id":0,"box":"dac~"},{"id":1,"box":"osc~","params":{"frequency":100}},{"id":2,"box":"osc~","params":{"frequency":101}},{"id":3,"box":"osc~","params":{"frequency":102}}],' +
        '"connections":[{"from":[1,0],"to":[0,0]},{"from":[2,0],"to":[0,1]},{"from":[3,0],"to":[0,0]}]}',
    );
    const frequencies = planPatch(2001).nodes.map((box) => box.params?.["frequency"]);
    expect([frequencies[1000], frequencies[1001], frequencies[1999]]).toStrictEqual([1099, 100, 1098]);
  });

  test("count a box as its patch minus the patch without it, in each precision given as binary32, and a plan as its load", () => {
    const blocks = (patch: Uint8Array, rate: number, format: "binary64-as-binary32" | "binary32"): bigint => {
      const instance = new CountedInstance(counted());
      instance.load(patch, rate, format);
      return instance.process(64);
    };
    const [first] = BOX_BENCHMARKS;
    if (first === undefined) {
      throw new Error("no benchmark");
    }
    const box = (format: "binary64-as-binary32" | "binary32"): number => Number(blocks(patchBytes(first.patch), 48_000, format) - blocks(patchBytes(first.without), 48_000, format));
    const counts = measure(counted(), [first], [3, 5]);
    expect(counts.boxes).toStrictEqual([
      { box: "osc~", precision: "binary64", setting: "440 Hz at 48 kHz", instructions: box("binary64-as-binary32") },
      { box: "osc~", precision: "binary32", setting: "440 Hz at 48 kHz", instructions: box("binary32") },
    ]);
    expect(counts.boxes[0]?.instructions).not.toBe(counts.boxes[1]?.instructions);
    expect(counts.plans).toStrictEqual(
      [3, 5].map((size) => ({ oscillators: size, instructions: Number(new CountedInstance(counted()).load(patchBytes(planPatch(size)), 48_000, "binary64-as-binary32")) })),
    );
    expect(counts.plans[0]?.instructions).toBeLessThan(counts.plans[1]?.instructions ?? 0);
  });

  test("refuse a box that costs less than nothing", () => {
    const [first] = BOX_BENCHMARKS;
    if (first === undefined) {
      throw new Error("no benchmark");
    }
    expect(() => measure(counted(), [{ ...first, patch: first.without, without: first.patch }], [])).toThrow(RangeError);
    expect(() => measure(counted(), [{ ...first, patch: first.without, without: first.patch }], [])).toThrow(/^a count of -\d+ instructions cannot be stored exactly$/);
  });

  test("accept a box that costs nothing", () => {
    const [first] = BOX_BENCHMARKS;
    if (first === undefined) {
      throw new Error("no benchmark");
    }
    expect(measure(counted(), [{ ...first, patch: first.without }], []).boxes.map((entry) => entry.instructions)).toStrictEqual([0, 0]);
  });

  test("refuse a patch the engine refuses", () => {
    const [first] = BOX_BENCHMARKS;
    if (first === undefined) {
      throw new Error("no benchmark");
    }
    expect(() => measure(counted(), [{ ...first, rate: 7 }], [])).toThrow(CountError);
    expect(() => measure(counted(), [], [0])).not.toThrow();
  });

  test("measure the committed baseline, within its tolerance, and build plans within n log n", () => {
    const baseline = parseBaseline(readFileSync(new URL("benchmark-baseline.json", SPEC), "utf8"));
    const thresholds = parseThresholds(readFileSync(new URL("benchmark-thresholds.json", SPEC), "utf8"));
    const counts = measure(counted(), BOX_BENCHMARKS, PLAN_SIZES);
    expect(regressions(counts, baseline, thresholds.tolerancePercent)).toStrictEqual([]);
    expect(nLogNProblems(counts.plans)).toStrictEqual([]);
  });
});

/** Counts of one box benchmark and one plan. */
function countsOf(box: number, plan: number): Counts {
  return { boxes: [{ box: "osc~", precision: "binary32", setting: "s", instructions: box }], plans: [{ oscillators: 4, instructions: plan }] };
}

describe("regressions", () => {
  test("accept counts within the tolerance, its edges included", () => {
    for (const [box, plan] of [
      [1000, 5000],
      [1020, 4900],
      [980, 5100],
    ]) {
      expect(regressions(countsOf(box ?? 0, plan ?? 0), countsOf(1000, 5000), 2)).toStrictEqual([]);
    }
  });

  test("refuse counts beyond the tolerance, either way", () => {
    expect(regressions(countsOf(1021, 4899), countsOf(1000, 5000), 2)).toStrictEqual([
      "osc~ s in binary32: 1021 instructions, baseline 1000 (+2.10%, beyond ±2%)",
      "loading a plan of 4 oscillators: 4899 instructions, baseline 5000 (-2.02%, beyond ±2%)",
    ]);
    expect(regressions(countsOf(1100, 5000), countsOf(1000, 5000), 15)).toStrictEqual([]);
    expect(regressions(countsOf(1000, 5000), countsOf(1100, 5000), 0.5)).toStrictEqual(["osc~ s in binary32: 1000 instructions, baseline 1100 (-9.09%, beyond ±0.5%)"]);
  });

  test("refuse a benchmark missing from the baseline, and a baseline entry nothing measures", () => {
    const other: Counts = { boxes: [{ box: "dac~", precision: "binary64", setting: "s", instructions: 7 }], plans: [{ oscillators: 8, instructions: 9 }] };
    const sameBoxOtherPrecision: Counts = { boxes: [{ box: "osc~", precision: "binary64", setting: "s", instructions: 1 }], plans: [{ oscillators: 4, instructions: 2 }] };
    expect(regressions(countsOf(1, 2), sameBoxOtherPrecision, 2)).toStrictEqual(["osc~ s in binary32: 1 instructions, not in the baseline", "osc~ s in binary64: in the baseline, measured by no benchmark"]);
    expect(regressions(countsOf(1, 2), other, 2)).toStrictEqual([
      "osc~ s in binary32: 1 instructions, not in the baseline",
      "loading a plan of 4 oscillators: 2 instructions, not in the baseline",
      "dac~ s in binary64: in the baseline, measured by no benchmark",
      "loading a plan of 8 oscillators: in the baseline, measured by no benchmark",
    ]);
  });
});

test("writes a change in percent signed, but zero", () => {
  expect([2.1, -0.5, 0, 12_345.678].map(signedPercent)).toStrictEqual(["+2.10", "-0.50", "0.00", "+12345.68"]);
});

describe("the n log n law", () => {
  /** Plans of these sizes, each costing `cost(size)`. */
  const plans = (cost: (size: number) => number): Counts["plans"] => [4, 16, 64, 256].map((size) => ({ oscillators: size, instructions: cost(size) }));

  test("accepts n log n with any non-negative linear and constant terms, at its edge", () => {
    for (const cost of [(size: number) => size * Math.log2(size), (size: number) => 7 * size * Math.log2(size) + 100 * size + 1000, (size: number) => 50 * size]) {
      expect(nLogNProblems(plans(cost))).toStrictEqual([]);
    }
    expect(nLogNProblems([])).toStrictEqual([]);
    expect(nLogNProblems([{ oscillators: 4, instructions: 8 }])).toStrictEqual([]);
  });

  test("refuses a quadratic cost, and a cost growing faster than n log n anywhere", () => {
    expect(nLogNProblems(plans((size) => size * size))).toStrictEqual([
      "building plans grows faster than n log n: 4 to 16 oscillators costs 16.000 times more, at most 8.000",
      "building plans grows faster than n log n: 16 to 64 oscillators costs 16.000 times more, at most 6.000",
      "building plans grows faster than n log n: 64 to 256 oscillators costs 16.000 times more, at most 5.333",
    ]);
    expect(
      nLogNProblems([
        { oscillators: 4, instructions: 8 },
        { oscillators: 16, instructions: 64 },
        { oscillators: 64, instructions: 385 },
      ]),
    ).toStrictEqual(["building plans grows faster than n log n: 16 to 64 oscillators costs 6.016 times more, at most 6.000"]);
  });
});

describe("the baseline file", () => {
  const valid: Counts = { boxes: [{ box: "osc~", precision: "binary32", setting: "s", instructions: 12 }], plans: [{ oscillators: 4, instructions: 0 }] };

  test("is read in its exact shape, and written as it is read", () => {
    expect(parseBaseline(JSON.stringify(valid))).toStrictEqual(valid);
    expect(formatBaseline(valid)).toBe(
      '{\n  "boxes": [\n    {\n      "box": "osc~",\n      "precision": "binary32",\n      "setting": "s",\n      "instructions": 12\n    }\n  ],\n  "plans": [\n    {\n      "oscillators": 4,\n      "instructions": 0\n    }\n  ]\n}\n',
    );
    expect(parseBaseline(formatBaseline(valid))).toStrictEqual(valid);
    const committed = readFileSync(new URL("benchmark-baseline.json", SPEC), "utf8");
    expect(formatBaseline(parseBaseline(committed))).toBe(committed);
  });

  test("refuses every other shape", () => {
    const refused = (value: unknown, message: string): void => {
      expect(() => parseBaseline(JSON.stringify(value))).toThrow(new BenchmarkFileError(message));
    };
    expect(() => parseBaseline("{")).toThrow(BenchmarkFileError);
    expect(() => parseBaseline("{")).toThrow(/^not JSON: SyntaxError/);
    const top = 'expected {"boxes": [...], "plans": [...]}';
    for (const value of [null, [], 3, {}, { boxes: [] }, { ...valid, extra: 1 }, { boxes: {}, plans: [] }, { boxes: [], plans: {} }]) {
      refused(value, top);
    }
    const box = 'boxes[0] is not {"box", "precision", "setting", "instructions"} with a whole count';
    const entries = [
      null,
      {},
      { box: 1, setting: "s", instructions: 1 },
      { box: "b", setting: 1, instructions: 1 },
      { box: "b", setting: "s", instructions: -1 },
      { box: "b", setting: "s", instructions: 1.5 },
      { box: "b", setting: "s", instructions: "1" },
      { box: "b", setting: "s", instructions: 2 ** 53 },
      { box: "b", setting: "s", instructions: 1, more: 1 },
    ];
    for (const entry of entries) {
      refused({ boxes: [entry], plans: [] }, box);
      if (entry !== null) {
        refused({ boxes: [{ ...entry, precision: "binary64" }], plans: [] }, box);
      }
    }
    refused({ boxes: [{ box: "b", setting: "s", instructions: 1 }], plans: [] }, box);
    for (const precision of ["binary16", 64, null]) {
      refused({ boxes: [{ box: "b", precision, setting: "s", instructions: 1 }], plans: [] }, "boxes[0].precision is not one of binary64, binary32");
    }
    const plan = 'plans[1] is not {"oscillators", "instructions"} with whole numbers';
    for (const entry of [{}, { oscillators: -1, instructions: 1 }, { oscillators: 1, instructions: -1 }, { oscillators: 1 }, { oscillators: 1, instructions: 1, more: 1 }]) {
      refused({ boxes: [], plans: [{ oscillators: 1, instructions: 1 }, entry] }, plan);
    }
  });

  test("names its errors", () => {
    expect(new BenchmarkFileError("x").name).toBe("BenchmarkFileError");
  });
});

describe("the thresholds file", () => {
  const valid = {
    tolerance_percent: 2,
    construction: { runs: 20, margin: 2.5, measured_on: "then", measured_max_ms: { chromium: 1.2, firefox: 4, webkit: 0.5 } },
    boxes: { blocks: 4096, runs: 5, margin: 3, measured_on: "later", measured_max_ms: { chromium: 10.1, firefox: 20, webkit: 7 } },
  };

  test("is read in its exact shape", () => {
    expect(parseThresholds(JSON.stringify(valid))).toStrictEqual({
      tolerancePercent: 2,
      construction: { runs: 20, margin: 2.5, measuredOn: "then", measuredMaxMs: { chromium: 1.2, firefox: 4, webkit: 0.5 } },
      boxes: { blocks: 4096, runs: 5, margin: 3, measuredOn: "later", measuredMaxMs: { chromium: 10.1, firefox: 20, webkit: 7 } },
    });
    expect(BROWSERS).toStrictEqual(["chromium", "firefox", "webkit"]);
  });

  test("gives each browser engine its bounds: the slowest time measured times the margin, rounded up", () => {
    const thresholds = parseThresholds(JSON.stringify(valid));
    expect(BROWSERS.map((browser) => constructionBoundMs(thresholds, browser))).toStrictEqual([3, 10, 2]);
    expect(BROWSERS.map((browser) => boxBoundMs(thresholds, browser))).toStrictEqual([31, 60, 21]);
  });

  test("refuses every other shape", () => {
    const refused = (value: unknown, message: string): void => {
      expect(() => parseThresholds(JSON.stringify(value))).toThrow(new BenchmarkFileError(message));
    };
    expect(() => parseThresholds("[")).toThrow(/^not JSON: SyntaxError/);
    const top = 'expected {"tolerance_percent": <positive>, "construction": {...}, "boxes": {...}}';
    const withoutBoxes = { tolerance_percent: valid.tolerance_percent, construction: valid.construction };
    for (const value of [null, {}, withoutBoxes, { ...valid, tolerance_percent: 0 }, { ...valid, tolerance_percent: -1 }, { ...valid, tolerance_percent: "2" }, { ...valid, tolerance_percent: Infinity }, { ...valid, extra: 1 }]) {
      refused(value, top);
    }
    const boxes = 'expected "boxes": {"blocks": <whole, positive>, "runs": <whole, positive>, "margin": <positive>, "measured_on": <text>, "measured_max_ms": {...}}';
    for (const change of [{ blocks: 0 }, { blocks: 1.5 }, { blocks: -1 }, { blocks: "1" }, { runs: 0 }, { margin: 0 }, { measured_on: 1 }, { extra: 1 }]) {
      refused({ ...valid, boxes: { ...valid.boxes, ...change } }, boxes);
    }
    const { runs, margin, measured_on, measured_max_ms } = valid.boxes;
    refused({ ...valid, boxes: { runs, margin, measured_on, measured_max_ms } }, boxes);
    refused({ ...valid, boxes: null }, boxes);
    refused({ ...valid, boxes: { ...valid.boxes, measured_max_ms: { chromium: 1, firefox: 1 } } }, 'expected "measured_max_ms" with exactly chromium, firefox, webkit');
    refused({ ...valid, boxes: { ...valid.boxes, measured_max_ms: { ...valid.boxes.measured_max_ms, webkit: 0 } } }, '"measured_max_ms"."webkit" is not a positive number');
    const construction = 'expected "construction": {"runs": <whole, positive>, "margin": <positive>, "measured_on": <text>, "measured_max_ms": {...}}';
    for (const change of [{ runs: 0 }, { runs: 1.5 }, { runs: -1 }, { margin: 0 }, { margin: "2" }, { measured_on: 1 }, { extra: 1 }]) {
      refused({ ...valid, construction: { ...valid.construction, ...change } }, construction);
    }
    refused({ ...valid, construction: null }, construction);
    refused({ ...valid, construction: { ...valid.construction, measured_max_ms: { chromium: 1, firefox: 1 } } }, 'expected "measured_max_ms" with exactly chromium, firefox, webkit');
    refused({ ...valid, construction: { ...valid.construction, measured_max_ms: { chromium: 1, firefox: 1, webkit: 1, edge: 1 } } }, 'expected "measured_max_ms" with exactly chromium, firefox, webkit');
    for (const browser of BROWSERS) {
      for (const value of [0, -1, "1", null]) {
        refused({ ...valid, construction: { ...valid.construction, measured_max_ms: { ...valid.construction.measured_max_ms, [browser]: value } } }, `"measured_max_ms"."${browser}" is not a positive number`);
      }
    }
  });
});
