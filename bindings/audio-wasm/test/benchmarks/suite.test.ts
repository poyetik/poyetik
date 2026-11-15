import { readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";
import { CountedInstance, CountError, countedEngine, type CountedEngine } from "../../benchmarks/counter.ts";
import {
  BLOCKS,
  BOX_BENCHMARKS,
  BROWSERS,
  BenchmarkFileError,
  boxBoundMs,
  boxName,
  constructionBoundMs,
  exact,
  forBenchmark,
  formatBaseline,
  measure,
  MIX_BENCHMARKS,
  mixPatch,
  nLogNProblems,
  parseBaseline,
  parseThresholds,
  PATCH_BENCHMARKS,
  patchName,
  PLAN_PRECISION,
  PLAN_RATE,
  PLAN_SIZES,
  planName,
  planPatch,
  patchBytes,
  regressions,
  signedPercent,
  summingPatch,
  type Counts,
} from "../../benchmarks/suite.ts";
import { rethrow, SPEC, wasmBytes } from "../fixtures.ts";
import { assemble, I32_GLOBAL } from "./assemble.ts";

/** The built engine module, instrumented and compiled at the first use (a mutant that breaks it fails a test, not the file). */
let compiled: CountedEngine | undefined;

/** {@link compiled}, compiled if needed. */
function counted(): CountedEngine {
  compiled ??= countedEngine(new Uint8Array(wasmBytes()));
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

  test("measure the plan's sums: cables into one inlet (one addition, partial and full trees, a lane of two terms) and output boxes on one channel", () => {
    expect(MIX_BENCHMARKS.map(({ setting, rate }) => [setting, rate])).toStrictEqual([
      ["2 cables into one inlet at 48 kHz", 48_000],
      ["3 cables into one inlet at 48 kHz", 48_000],
      ["16 cables into one inlet at 48 kHz", 48_000],
      ["17 cables into one inlet at 48 kHz", 48_000],
      ["two output boxes fed by one oscillator at 48 kHz", 48_000],
    ]);
    const text = (patch: Parameters<typeof patchBytes>[0]): string => new TextDecoder().decode(patchBytes(patch));
    const [, three, , , outputs] = MIX_BENCHMARKS;
    expect([three, outputs].map((benchmark) => [text(benchmark?.patch ?? mixPatch(0, 0)), text(benchmark?.without ?? mixPatch(0, 0))])).toStrictEqual([
      [
        '{"format":1,"nodes":[{"id":0,"box":"dac~"},{"id":1,"box":"osc~","params":{"frequency":137}},{"id":2,"box":"osc~","params":{"frequency":174}},{"id":3,"box":"osc~","params":{"frequency":211}}],' +
          '"connections":[{"from":[1,0],"to":[0,0]},{"from":[2,0],"to":[0,0]},{"from":[3,0],"to":[0,0]}]}',
        '{"format":1,"nodes":[{"id":0,"box":"dac~"},{"id":1,"box":"osc~","params":{"frequency":137}},{"id":2,"box":"osc~","params":{"frequency":174}},{"id":3,"box":"osc~","params":{"frequency":211}}],' +
          '"connections":[{"from":[1,0],"to":[0,0]}]}',
      ],
      [
        '{"format":1,"nodes":[{"id":1,"box":"osc~","params":{"frequency":440}},{"id":2,"box":"dac~"},{"id":3,"box":"dac~"}],"connections":[{"from":[1,0],"to":[2,0]},{"from":[1,0],"to":[3,0]}]}',
        '{"format":1,"nodes":[{"id":1,"box":"osc~","params":{"frequency":440}},{"id":2,"box":"dac~"}],"connections":[{"from":[1,0],"to":[2,0]}]}',
      ],
    ]);
    for (const [index, cables] of [2, 3, 16, 17].entries()) {
      expect(MIX_BENCHMARKS[index]).toStrictEqual({ setting: `${String(cables)} cables into one inlet at 48 kHz`, rate: 48_000, patch: mixPatch(cables, cables), without: mixPatch(cables, 1) });
    }
  });

  test("play whole patches: the empty patch, and 16 output boxes each fed by 3 oscillators", () => {
    const text = (patch: Parameters<typeof patchBytes>[0]): string => new TextDecoder().decode(patchBytes(patch));
    expect(PATCH_BENCHMARKS.map(({ setting, rate, patch }) => [setting, rate, text(patch)])).toStrictEqual([
      ["an empty patch at 48 kHz", 48_000, '{"format":1,"nodes":[],"connections":[]}'],
      ["16 output boxes each fed by 3 oscillators at 48 kHz", 48_000, text(summingPatch(16, 3))],
    ]);
    expect(text(summingPatch(2, 2))).toBe(
      '{"format":1,"nodes":[{"id":1,"box":"dac~"},{"id":2,"box":"osc~","params":{"frequency":102}},{"id":3,"box":"osc~","params":{"frequency":103}},' +
        '{"id":4,"box":"dac~"},{"id":5,"box":"osc~","params":{"frequency":105}},{"id":6,"box":"osc~","params":{"frequency":106}}],' +
        '"connections":[{"from":[2,0],"to":[1,0]},{"from":[2,0],"to":[1,1]},{"from":[3,0],"to":[1,0]},{"from":[5,0],"to":[4,0]},{"from":[5,0],"to":[4,1]},{"from":[6,0],"to":[4,0]}]}',
    );
    const heavy = summingPatch(16, 3);
    expect([heavy.nodes.length, heavy.nodes.filter((node) => node.box === "dac~").length, heavy.connections.length]).toStrictEqual([64, 16, 64]);
    expect(heavy.nodes.at(-1)).toStrictEqual({ id: 64, box: "osc~", params: { frequency: 164 } });
    expect(heavy.connections.slice(-4)).toStrictEqual([
      { from: [62, 0], to: [61, 0] },
      { from: [62, 0], to: [61, 1] },
      { from: [63, 0], to: [61, 0] },
      { from: [64, 0], to: [61, 0] },
    ]);
    expect(text(summingPatch(1, 0))).toBe('{"format":1,"nodes":[{"id":1,"box":"dac~"}],"connections":[]}');
    expect(text(summingPatch(0, 3))).toBe('{"format":1,"nodes":[],"connections":[]}');
  });

  test("build plans of oscillators feeding both inlets of one output in turn", () => {
    expect(new TextDecoder().decode(patchBytes(planPatch(3)))).toBe(
      '{"format":1,"nodes":[{"id":0,"box":"dac~"},{"id":1,"box":"osc~","params":{"frequency":100}},{"id":2,"box":"osc~","params":{"frequency":101}},{"id":3,"box":"osc~","params":{"frequency":102}}],' +
        '"connections":[{"from":[1,0],"to":[0,0]},{"from":[2,0],"to":[0,1]},{"from":[3,0],"to":[0,0]}]}',
    );
    const frequencies = planPatch(2001).nodes.map((box) => box.params?.["frequency"]);
    expect([frequencies[1000], frequencies[1001], frequencies[1999]]).toStrictEqual([1099, 100, 1098]);
  });

  test("count a box as its patch minus the patch without it, in each precision given as binary32, a whole patch as it is, and a plan as its load", () => {
    const blocks = (patch: Uint8Array, rate: number, format: "binary64-as-binary32" | "binary32"): bigint => {
      const instance = new CountedInstance(counted().module);
      instance.load(patch, rate, format);
      return instance.process(64);
    };
    const [first] = BOX_BENCHMARKS;
    if (first === undefined) {
      throw new Error("no benchmark");
    }
    const box = (format: "binary64-as-binary32" | "binary32"): number => Number(blocks(patchBytes(first.patch), 48_000, format) - blocks(patchBytes(first.without), 48_000, format));
    const [mix] = MIX_BENCHMARKS;
    if (mix === undefined) {
      throw new Error("no mixing benchmark");
    }
    const mixing = (format: "binary64-as-binary32" | "binary32"): number => Number(blocks(patchBytes(mix.patch), 48_000, format) - blocks(patchBytes(mix.without), 48_000, format));
    const [, heavy] = PATCH_BENCHMARKS;
    if (heavy === undefined) {
      throw new Error("no whole-patch benchmark");
    }
    const counts = measure(counted(), [first], [mix], [heavy], [3, 5]);
    expect(counts.patches).toStrictEqual([
      { precision: "binary64", setting: "16 output boxes each fed by 3 oscillators at 48 kHz", instructions: Number(blocks(patchBytes(heavy.patch), 48_000, "binary64-as-binary32")) },
      { precision: "binary32", setting: "16 output boxes each fed by 3 oscillators at 48 kHz", instructions: Number(blocks(patchBytes(heavy.patch), 48_000, "binary32")) },
    ]);
    expect(counts.patches[0]?.instructions).not.toBe(counts.patches[1]?.instructions);
    expect(counts.mixes).toStrictEqual([
      { precision: "binary64", setting: "2 cables into one inlet at 48 kHz", instructions: mixing("binary64-as-binary32") },
      { precision: "binary32", setting: "2 cables into one inlet at 48 kHz", instructions: mixing("binary32") },
    ]);
    expect(counts.mixes[0]?.instructions).toBeGreaterThan(0);
    expect(counts.boxes).toStrictEqual([
      { box: "osc~", precision: "binary64", setting: "440 Hz at 48 kHz", instructions: box("binary64-as-binary32") },
      { box: "osc~", precision: "binary32", setting: "440 Hz at 48 kHz", instructions: box("binary32") },
    ]);
    expect(counts.boxes[0]?.instructions).not.toBe(counts.boxes[1]?.instructions);
    expect(counts.plans).toStrictEqual(
      [3, 5].map((size) => ({ oscillators: size, instructions: Number(new CountedInstance(counted().module).load(patchBytes(planPatch(size)), 48_000, "binary64-as-binary32")) })),
    );
    expect(counts.plans[0]?.instructions).toBeLessThan(counts.plans[1]?.instructions ?? 0);
    expect(counts.moduleBytes).toBe(wasmBytes().byteLength);
  });

  test("refuse a mix that costs less than nothing, and a mixing patch the engine refuses", () => {
    const [mix] = MIX_BENCHMARKS;
    if (mix === undefined) {
      throw new Error("no mixing benchmark");
    }
    expect(() => measure(counted(), [], [{ ...mix, patch: mix.without, without: mix.patch }], [], [])).toThrow(
      /^mixing 2 cables into one inlet at 48 kHz in binary64: the patch without it costs \d+ instructions more than the patch holding it$/,
    );
    expect(() => measure(counted(), [], [{ ...mix, rate: 7 }], [], [])).toThrow(CountError);
    expect(() => measure(counted(), [], [{ ...mix, rate: 7 }], [], [])).toThrow(/^mixing 2 cables into one inlet at 48 kHz in binary64: the engine refused the patch: ./);
  });

  test("refuse a box that costs less than nothing", () => {
    const [first] = BOX_BENCHMARKS;
    if (first === undefined) {
      throw new Error("no benchmark");
    }
    expect(() => measure(counted(), [{ ...first, patch: first.without, without: first.patch }], [], [], [])).toThrow(RangeError);
    expect(() => measure(counted(), [{ ...first, patch: first.without, without: first.patch }], [], [], [])).toThrow(
      /^osc~ 440 Hz at 48 kHz in binary64: the patch without it costs \d+ instructions more than the patch holding it$/,
    );
  });

  test("refuse two benchmarks of one name, which the baseline could not tell apart", () => {
    const [first] = BOX_BENCHMARKS;
    const [mix] = MIX_BENCHMARKS;
    if (first === undefined || mix === undefined) {
      throw new Error("no benchmark");
    }
    expect(() => measure(counted(), [first, { ...first, rate: 8000 }], [], [], [])).toThrow(new RangeError("osc~ 440 Hz at 48 kHz is measured twice"));
    expect(() => measure(counted(), [], [mix, mix], [], [])).toThrow(new RangeError("mixing 2 cables into one inlet at 48 kHz is measured twice"));
    expect(() => measure(counted(), [], [], [], [3, 5, 3])).toThrow(new RangeError("loading a plan of 3 oscillators is measured twice"));
    const [empty] = PATCH_BENCHMARKS;
    if (empty === undefined) {
      throw new Error("no whole-patch benchmark");
    }
    expect(() => measure(counted(), [], [], [empty, { ...empty, rate: 8000 }], [])).toThrow(new RangeError("playing an empty patch at 48 kHz is measured twice"));
    expect(measure(counted(), [], [], [empty, { ...empty, setting: "other" }], []).patches).toHaveLength(4);
    expect(measure(counted(), [first, { ...first, setting: "other" }], [], [], [3, 5]).boxes).toHaveLength(4);
  });

  test("name the benchmark whose measurement fails, keeping the cause", () => {
    const cause = new Error("inner");
    const failure = ((): unknown => {
      try {
        return forBenchmark("loading a plan of 2 oscillators", () => {
          throw cause;
        });
      } catch (error) {
        return error;
      }
    })();
    expect(failure).toStrictEqual(new CountError("loading a plan of 2 oscillators: inner"));
    expect(failure instanceof CountError && failure.cause).toBe(cause);
    expect(() => forBenchmark("b", () => rethrow("busy"))).toThrow(new CountError("b: busy"));
    expect(forBenchmark("b", () => 3)).toBe(3);
    expect(() => measure(counted(), [], [], [], [2 ** 32])).toThrow(new CountError("loading a plan of 4294967296 oscillators: Invalid array length"));
  });

  test("hold a count JSON keeps exactly, and refuse a larger one", () => {
    expect(exact(2n ** 53n - 1n)).toBe(9_007_199_254_740_991);
    expect(exact(0n)).toBe(0);
    expect(() => exact(2n ** 53n)).toThrow(new RangeError("a count of 9007199254740992 instructions cannot be stored exactly"));
  });

  test("name each benchmark as the reports do", () => {
    expect(boxName({ box: "osc~", precision: "binary32", setting: "s" })).toBe("osc~ s in binary32");
    expect(planName(256)).toBe("loading a plan of 256 oscillators");
    expect(patchName({ precision: "binary64", setting: "p" })).toBe("playing p in binary64");
  });

  test("accept a box that costs nothing", () => {
    const [first] = BOX_BENCHMARKS;
    if (first === undefined) {
      throw new Error("no benchmark");
    }
    expect(measure(counted(), [{ ...first, patch: first.without }], [], [], []).boxes.map((entry) => entry.instructions)).toStrictEqual([0, 0]);
  });

  test("refuse a patch the engine refuses", () => {
    const [first] = BOX_BENCHMARKS;
    if (first === undefined) {
      throw new Error("no benchmark");
    }
    expect(() => measure(counted(), [{ ...first, rate: 7 }], [], [], [])).toThrow(CountError);
    expect(() => measure(counted(), [], [], [], [0])).not.toThrow();
    const [empty] = PATCH_BENCHMARKS;
    if (empty === undefined) {
      throw new Error("no whole-patch benchmark");
    }
    expect(() => measure(counted(), [], [], [{ ...empty, rate: 7 }], [])).toThrow(new CountError("playing an empty patch at 48 kHz in binary64: the engine refused the patch: invalid sample rate: 7 Hz (supported: 8000 to 384000 Hz)"));
  });

  test("measure the committed baseline, within its tolerance, and build plans within n log n", () => {
    const baseline = parseBaseline(readFileSync(new URL("benchmark-baseline.json", SPEC), "utf8"));
    const thresholds = parseThresholds(readFileSync(new URL("benchmark-thresholds.json", SPEC), "utf8"));
    const counts = measure(counted(), BOX_BENCHMARKS, MIX_BENCHMARKS, PATCH_BENCHMARKS, PLAN_SIZES);
    expect(regressions(counts, baseline, thresholds.tolerancePercent)).toStrictEqual([]);
    expect(nLogNProblems(counts.plans)).toStrictEqual([]);
  });
});

/**
 * A stand-in engine module, assembled by hand, of the engine's interface: `engine_input`
 * keeps the length of the patch, `engine_load` accepts it, and each `engine_process` runs
 * `fixed` `nop`s, then a loop of one pass per byte of the patch. So a block costs a part
 * that grows with the patch, which differences see, plus `fixed` instructions every patch
 * pays, which they cancel.
 */
function standIn(fixed: number): CountedEngine {
  const constant = (bytes: readonly number[]): readonly number[] => [0x41, ...bytes, 0x0b];
  const func = (exported: string, params: number, code: readonly number[], locals: readonly number[] = [0x00]): Parameters<typeof assemble>[0]["functions"][number] => ({ params, result: true, locals, code, exported });
  const pass = [0x03, 0x40, 0x20, 0x00, 0x45, 0x0d, 0x01, 0x20, 0x00, 0x41, 0x01, 0x6b, 0x21, 0x00, 0x0c, 0x00, 0x0b];
  const wasm = assemble({
    functions: [
      // 0x5059_0005 in signed LEB128: the interface's identity and version.
      func("engine_abi_version", 0, constant([0x85, 0x80, 0xe4, 0x82, 0x05])),
      func("engine_input", 1, [0x20, 0x00, 0x24, 0x00, ...constant([0x00])]),
      func("engine_load", 2, constant([0x01])),
      func("engine_message_ptr", 0, constant([0x00])),
      func("engine_message_len", 0, constant([0x00])),
      func("engine_channels", 0, constant([0x02])),
      func("engine_block_size", 0, constant([0x10])),
      func("engine_process", 0, [...Array<number>(fixed).fill(0x01), 0x23, 0x00, 0x21, 0x00, 0x02, 0x40, ...pass, 0x0b, ...constant([0x00])], [0x01, 0x01, 0x7f]),
      func("engine_max_patch_bytes", 0, constant([0x00])),
      func("engine_subnormals", 0, constant([0x0f])),
      // Its report at address 0, never read by the benchmarks.
      func("engine_replaced", 0, constant([0x00])),
    ],
    globals: [I32_GLOBAL],
    exports: [["memory", 0x02, 0]],
  });
  return countedEngine(wasm);
}

test("the whole-patch benchmarks see a cost every patch pays each block, which every difference cancels", () => {
  const suite = [BOX_BENCHMARKS, MIX_BENCHMARKS, PATCH_BENCHMARKS, [2, 4]] as const;
  const without = measure(standIn(0), ...suite);
  const fixed = measure(standIn(5), ...suite);
  // The positive control: the patches' counts differ by the 5 instructions of each block.
  expect(fixed.patches).toStrictEqual(without.patches.map((entry) => ({ ...entry, instructions: entry.instructions + 5 * BLOCKS })));
  // And the differences do not see them.
  expect([fixed.boxes, fixed.mixes, fixed.plans]).toStrictEqual([without.boxes, without.mixes, without.plans]);
  expect(without.boxes.every((entry) => entry.instructions > 0) && without.mixes.every((entry) => entry.instructions > 0)).toBe(true);
  // Nothing subtracted: the empty patch's 40 bytes cost 8 instructions a pass, plus 2 before
  // the loop, 3 in its last test and 1 for the result, in each of the 64 blocks.
  const [empty, , heavy] = without.patches;
  expect(empty).toStrictEqual({ precision: "binary64", setting: "an empty patch at 48 kHz", instructions: 64 * (8 * 40 + 6) });
  // And the heavy patch's 4,569 bytes, at the same 8 instructions a pass.
  expect(heavy).toStrictEqual({ precision: "binary64", setting: "16 output boxes each fed by 3 oscillators at 48 kHz", instructions: 64 * (8 * 4569 + 6) });
});

/** Counts of one box benchmark and one plan. */
function countsOf(box: number, plan: number, mix = 100, moduleBytes = 300_000, patch = 5000): Counts {
  return {
    boxes: [{ box: "osc~", precision: "binary32", setting: "s", instructions: box }],
    mixes: [{ precision: "binary64", setting: "m", instructions: mix }],
    patches: [{ precision: "binary32", setting: "p", instructions: patch }],
    plans: [{ oscillators: 4, instructions: plan }],
    moduleBytes,
  };
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
    expect(regressions(countsOf(1000, 5000, 103), countsOf(1000, 5000, 100), 2)).toStrictEqual(["mixing m in binary64: 103 instructions, baseline 100 (+3.00%, beyond ±2%)"]);
    expect(regressions(countsOf(1000, 5000, 100, 300_000, 5101), countsOf(1000, 5000), 2)).toStrictEqual(["playing p in binary32: 5101 instructions, baseline 5000 (+2.02%, beyond ±2%)"]);
    expect(regressions(countsOf(1000, 5000, 100, 300_000, 4899), countsOf(1000, 5000), 2)).toStrictEqual(["playing p in binary32: 4899 instructions, baseline 5000 (-2.02%, beyond ±2%)"]);
    expect(regressions(countsOf(1000, 5000, 100, 300_000, 5100), countsOf(1000, 5000), 2)).toStrictEqual([]);
    expect(regressions(countsOf(1100, 5000), countsOf(1000, 5000), 15)).toStrictEqual([]);
    expect(regressions(countsOf(1000, 5000), countsOf(1100, 5000), 0.5)).toStrictEqual(["osc~ s in binary32: 1000 instructions, baseline 1100 (-9.09%, beyond ±0.5%)"]);
  });

  test("compare the size of the engine module the same way, its edges included", () => {
    for (const size of [300_000, 306_000, 294_000]) {
      expect(regressions(countsOf(1000, 5000, 100, size), countsOf(1000, 5000), 2)).toStrictEqual([]);
    }
    expect(regressions(countsOf(1000, 5000, 100, 309_000), countsOf(1000, 5000), 2)).toStrictEqual(["engine module: 309000 bytes, baseline 300000 (+3.00%, beyond ±2%)"]);
    expect(regressions(countsOf(1000, 5000, 100, 293_999), countsOf(1000, 5000), 2)).toStrictEqual(["engine module: 293999 bytes, baseline 300000 (-2.00%, beyond ±2%)"]);
  });

  test("refuse a benchmark missing from the baseline, and a baseline entry nothing measures", () => {
    const other: Counts = {
      boxes: [{ box: "dac~", precision: "binary64", setting: "s", instructions: 7 }],
      mixes: [{ precision: "binary32", setting: "m", instructions: 100 }],
      patches: [{ precision: "binary64", setting: "p", instructions: 5000 }],
      plans: [{ oscillators: 8, instructions: 9 }],
      moduleBytes: 300_000,
    };
    const sameBoxOtherPrecision: Counts = {
      boxes: [{ box: "osc~", precision: "binary64", setting: "s", instructions: 1 }],
      mixes: [{ precision: "binary64", setting: "m", instructions: 100 }],
      patches: [{ precision: "binary32", setting: "p", instructions: 5000 }],
      plans: [{ oscillators: 4, instructions: 2 }],
      moduleBytes: 300_000,
    };
    expect(regressions(countsOf(1, 2), sameBoxOtherPrecision, 2)).toStrictEqual(["osc~ s in binary32: 1 instructions, not in the baseline", "osc~ s in binary64: in the baseline, measured by no benchmark"]);
    expect(regressions(countsOf(1, 2), other, 2)).toStrictEqual([
      "osc~ s in binary32: 1 instructions, not in the baseline",
      "mixing m in binary64: 100 instructions, not in the baseline",
      "playing p in binary32: 5000 instructions, not in the baseline",
      "loading a plan of 4 oscillators: 2 instructions, not in the baseline",
      "dac~ s in binary64: in the baseline, measured by no benchmark",
      "mixing m in binary32: in the baseline, measured by no benchmark",
      "playing p in binary64: in the baseline, measured by no benchmark",
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
    expect(
      nLogNProblems([
        { oscillators: 2, instructions: 2 },
        { oscillators: 4, instructions: 8 },
      ]),
    ).toStrictEqual([]);
  });

  test("refuses plans outside its domain instead of passing them", () => {
    expect(
      nLogNProblems([
        { oscillators: 1, instructions: 10 },
        { oscillators: 4, instructions: 1000 },
      ]),
    ).toStrictEqual(["the n log n law needs plans of 2 oscillators or more that cost something: loading a plan of 1 oscillators costs 10 instructions"]);
    expect(
      nLogNProblems([
        { oscillators: 0, instructions: 0 },
        { oscillators: 4, instructions: 1 },
      ]),
    ).toStrictEqual(["the n log n law needs plans of 2 oscillators or more that cost something: loading a plan of 0 oscillators costs 0 instructions"]);
    expect(
      nLogNProblems([
        { oscillators: 4, instructions: 0 },
        { oscillators: 16, instructions: 0 },
      ]),
    ).toStrictEqual([
      "the n log n law needs plans of 2 oscillators or more that cost something: loading a plan of 4 oscillators costs 0 instructions",
      "the n log n law needs plans of 2 oscillators or more that cost something: loading a plan of 16 oscillators costs 0 instructions",
    ]);
    for (const second of [4, 2]) {
      expect(
        nLogNProblems([
          { oscillators: 4, instructions: 8 },
          { oscillators: second, instructions: 8 },
        ]),
      ).toStrictEqual([`the n log n law needs plans in increasing size: ${String(second)} oscillators after 4`]);
    }
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
  const valid: Counts = {
    boxes: [{ box: "osc~", precision: "binary32", setting: "s", instructions: 12 }],
    mixes: [{ precision: "binary64", setting: "m", instructions: 3 }],
    patches: [{ precision: "binary32", setting: "p", instructions: 7 }],
    plans: [{ oscillators: 4, instructions: 0 }],
    moduleBytes: 327_000,
  };

  test("is read in its exact shape, and written as it is read", () => {
    expect(parseBaseline('{"boxes":[{"box":"osc~","precision":"binary32","setting":"s","instructions":12}],"mixes":[{"precision":"binary64","setting":"m","instructions":3}],"patches":[{"precision":"binary32","setting":"p","instructions":7}],"plans":[{"oscillators":4,"instructions":0}],"module_bytes":327000}')).toStrictEqual(valid);
    expect(formatBaseline(valid)).toBe(
      '{\n  "boxes": [\n    {\n      "box": "osc~",\n      "precision": "binary32",\n      "setting": "s",\n      "instructions": 12\n    }\n  ],\n  "mixes": [\n    {\n      "precision": "binary64",\n      "setting": "m",\n      "instructions": 3\n    }\n  ],\n  "patches": [\n    {\n      "precision": "binary32",\n      "setting": "p",\n      "instructions": 7\n    }\n  ],\n  "plans": [\n    {\n      "oscillators": 4,\n      "instructions": 0\n    }\n  ],\n  "module_bytes": 327000\n}\n',
    );
    expect(parseBaseline(formatBaseline(valid))).toStrictEqual(valid);
    const committed = readFileSync(new URL("benchmark-baseline.json", SPEC), "utf8");
    expect(formatBaseline(parseBaseline(committed))).toBe(committed);
    const otherPrecision: Counts = { ...valid, boxes: [...valid.boxes, { ...valid.boxes[0], box: "osc~", setting: "s", instructions: 1, precision: "binary64" }] };
    expect(parseBaseline(formatBaseline(otherPrecision))).toStrictEqual(otherPrecision);
  });

  test("refuses an entry repeated, which would hide the first", () => {
    const [box] = valid.boxes;
    const [mix] = valid.mixes;
    const [plan] = valid.plans;
    const [patch] = valid.patches;
    if (box === undefined || mix === undefined || plan === undefined || patch === undefined) {
      throw new Error("no entry");
    }
    expect(() => parseBaseline(formatBaseline({ ...valid, boxes: [box, { ...box, instructions: 1 }] }))).toThrow(new BenchmarkFileError('boxes[1] repeats "osc~ s in binary32"'));
    expect(() => parseBaseline(formatBaseline({ ...valid, mixes: [mix, mix] }))).toThrow(new BenchmarkFileError('mixes[1] repeats "mixing m in binary64"'));
    expect(() => parseBaseline(formatBaseline({ ...valid, patches: [patch, { ...patch, precision: "binary64" }, { ...patch, instructions: 1 }] }))).toThrow(new BenchmarkFileError('patches[2] repeats "playing p in binary32"'));
    expect(() => parseBaseline(formatBaseline({ ...valid, plans: [plan, { oscillators: 8, instructions: 1 }, plan] }))).toThrow(new BenchmarkFileError('plans[2] repeats "loading a plan of 4 oscillators"'));
  });

  test("refuses a key named __proto__, which JSON.parse keeps as its own", () => {
    expect(() => parseBaseline('{"boxes":[],"mixes":[],"patches":[],"plans":[],"module_bytes":1,"__proto__":{"x":1}}')).toThrow(new BenchmarkFileError('expected {"boxes": [...], "mixes": [...], "patches": [...], "plans": [...], "module_bytes": <whole>}'));
    expect(() => parseBaseline('{"boxes":[],"mixes":[],"patches":[],"plans":[{"oscillators":4,"instructions":1,"__proto__":1}],"module_bytes":1}')).toThrow(
      new BenchmarkFileError('plans[0] is not {"oscillators", "instructions"} with whole numbers'),
    );
  });

  test("refuses every other shape", () => {
    const refused = (value: unknown, message: string): void => {
      expect(() => parseBaseline(JSON.stringify(value))).toThrow(new BenchmarkFileError(message));
    };
    expect(() => parseBaseline("{")).toThrow(BenchmarkFileError);
    expect(() => parseBaseline("{")).toThrow(/^not JSON: SyntaxError/);
    const top = 'expected {"boxes": [...], "mixes": [...], "patches": [...], "plans": [...], "module_bytes": <whole>}';
    const empty = { boxes: [], mixes: [], patches: [], plans: [], module_bytes: 1 };
    for (const value of [
      null,
      [],
      3,
      {},
      { boxes: [] },
      { boxes: [], mixes: [], patches: [], plans: [] },
      { boxes: [], mixes: [], plans: [], module_bytes: 1 },
      { ...empty, extra: 1 },
      { ...empty, boxes: {} },
      { ...empty, mixes: {} },
      { ...empty, patches: {} },
      { ...empty, plans: {} },
      { ...empty, module_bytes: -1 },
      { ...empty, module_bytes: 1.5 },
      { ...empty, module_bytes: "1" },
    ]) {
      refused(value, top);
    }
    const mix = 'mixes[0] is not {"precision", "setting", "instructions"} with a whole count';
    for (const entry of [null, {}, { setting: 1, instructions: 1 }, { setting: "m", instructions: -1 }, { setting: "m", instructions: 1.5 }, { setting: "m", instructions: 1, more: 1 }, { setting: "m" }]) {
      refused({ boxes: [], mixes: [entry], patches: [], plans: [], module_bytes: 1 }, mix);
      if (entry !== null) {
        refused({ boxes: [], mixes: [{ ...entry, precision: "binary32" }], patches: [], plans: [], module_bytes: 1 }, mix);
      }
    }
    for (const precision of ["binary16", 64, null]) {
      refused({ boxes: [], mixes: [{ precision, setting: "m", instructions: 1 }], patches: [], plans: [], module_bytes: 1 }, "mixes[0].precision is not one of binary64, binary32");
    }
    const patch = 'patches[0] is not {"precision", "setting", "instructions"} with a whole count';
    for (const entry of [null, {}, { setting: 1, instructions: 1 }, { setting: "p", instructions: -1 }, { setting: "p", instructions: 1, more: 1 }]) {
      refused({ boxes: [], mixes: [], patches: [entry], plans: [], module_bytes: 1 }, patch);
      if (entry !== null) {
        refused({ boxes: [], mixes: [], patches: [{ ...entry, precision: "binary64" }], plans: [], module_bytes: 1 }, patch);
      }
    }
    refused({ boxes: [], mixes: [], patches: [{ precision: "binary16", setting: "p", instructions: 1 }], plans: [], module_bytes: 1 }, "patches[0].precision is not one of binary64, binary32");
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
      refused({ boxes: [entry], mixes: [], patches: [], plans: [], module_bytes: 1 }, box);
      if (entry !== null) {
        refused({ boxes: [{ ...entry, precision: "binary64" }], mixes: [], patches: [], plans: [], module_bytes: 1 }, box);
      }
    }
    refused({ boxes: [{ box: "b", setting: "s", instructions: 1 }], mixes: [], patches: [], plans: [], module_bytes: 1 }, box);
    for (const precision of ["binary16", 64, null]) {
      refused({ boxes: [{ box: "b", precision, setting: "s", instructions: 1 }], mixes: [], patches: [], plans: [], module_bytes: 1 }, "boxes[0].precision is not one of binary64, binary32");
    }
    const plan = 'plans[1] is not {"oscillators", "instructions"} with whole numbers';
    for (const entry of [{}, { oscillators: -1, instructions: 1 }, { oscillators: 1, instructions: -1 }, { oscillators: 1 }, { oscillators: 1, instructions: 1, more: 1 }]) {
      refused({ boxes: [], mixes: [], patches: [], plans: [{ oscillators: 1, instructions: 1 }, entry], module_bytes: 1 }, plan);
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
    const top = 'expected {"tolerance_percent": <positive, below 100>, "construction": {...}, "boxes": {...}}';
    const withoutBoxes = { tolerance_percent: valid.tolerance_percent, construction: valid.construction };
    for (const value of [
      null,
      {},
      withoutBoxes,
      { ...valid, tolerance_percent: 0 },
      { ...valid, tolerance_percent: -1 },
      { ...valid, tolerance_percent: "2" },
      { ...valid, tolerance_percent: Infinity },
      { ...valid, tolerance_percent: 100 },
      { ...valid, extra: 1 },
    ]) {
      refused(value, top);
    }
    expect(parseThresholds(JSON.stringify({ ...valid, tolerance_percent: 99.99 })).tolerancePercent).toBe(99.99);
    const text = JSON.stringify(valid);
    expect(() => parseThresholds(`${text.slice(0, -1)},"__proto__":{}}`)).toThrow(new BenchmarkFileError(top));
    expect(() => parseThresholds(text.replace('"webkit":0.5}', '"webkit":0.5,"__proto__":1}'))).toThrow(new BenchmarkFileError('expected "measured_max_ms" with exactly chromium, firefox, webkit'));
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
