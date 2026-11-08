import { describe, expect, test } from "vitest";
import { renderHash } from "../src/hash.ts";
import { judge, type Corpus, type Verdict } from "../src/oracle.ts";
import { parseWav } from "../src/wav.ts";
import { makeWav, sineSamples } from "./fixtures.ts";

/** Sample rate of the test case, in Hz. */
const RATE = 8000;
/** Length of the test case, in frames per channel. */
const FRAMES = 64;
/** The expected signal of each of the two channels of `WAV`. */
const CHANNEL = { sine: { frequency: 1000, amplitude: 1, phase: 0 } };
/** The exported file of the passing case: a 1 kHz sine on two channels. */
const WAV = makeWav({ rate: RATE, channels: 2, samples: sineSamples(2, FRAMES, RATE, 1000) });
/** The reference hash of `WAV`, so that the unchanged corpus passes. */
const HASH = renderHash(parseWav(WAV));

/** A corpus with one passing case, `sine`, changed by `change`. */
function corpus(change: Partial<Corpus> = {}): Corpus {
  return {
    cases: new Map([["sine", { sample_rate: RATE, frames: FRAMES }]]),
    references: { sine: HASH },
    expectations: { sine: { sample_rate: RATE, frames: FRAMES, channels: [CHANNEL, CHANNEL] } },
    wavs: new Map([["sine", WAV]]),
    ...change,
  };
}

describe("judge", () => {
  test("passes a case whose file matches its reference and its expected signal", () => {
    const verdict = judge(corpus());
    expect(verdict.failures).toBe(0);
    expect(verdict.lines).toHaveLength(1);
    expect(verdict.lines[0]).toMatch(new RegExp(`^ok {7}sine ${HASH} \\(2 channel\\(s\\), sine error \\d\\.\\d{2}e-\\d+\\)$`));
  });

  test("reports the largest error of all channels, accepted up to the bound", () => {
    // Channel 1, sample 0: the ideal is 0 and the sample is 2^-25, the output-rounding
    // term of the bound: accepted, and larger than any rounding error of channel 0.
    const samples = sineSamples(2, FRAMES, RATE, 1000);
    samples[1] = 2 ** -25;
    const wav = makeWav({ rate: RATE, channels: 2, samples });
    const verdict = judge(corpus({ wavs: new Map([["sine", wav]]), references: { sine: renderHash(parseWav(wav)) } }));
    expect(verdict.failures).toBe(0);
    expect(verdict.lines[0]).toContain(`sine error ${(2 ** -25).toExponential(2)}`);
  });

  test("reports a wrong hash", () => {
    const verdict = judge(corpus({ references: { sine: "0".repeat(64) } }));
    expect(verdict).toStrictEqual({
      lines: [`FAIL     sine: hash: the WAV file hashes to ${HASH}, the reference is ${"0".repeat(64)}`],
      failures: 1,
    });
  });

  test("reports a signal off the expected sine, channel by channel", () => {
    const off = { ...CHANNEL, sine: { ...CHANNEL.sine, frequency: 1001 } };
    const verdict = judge(corpus({ expectations: { sine: { sample_rate: RATE, frames: FRAMES, channels: [CHANNEL, off] } } }));
    expect(verdict.failures).toBe(1);
    expect(verdict.lines[0]).toMatch(/^FAIL {5}sine: channel 1: off the expected sine by \d\.\d{2}e-\d+ \(at most 2\.98e-8\)$/);
  });

  test("fails a signal that is not finite", () => {
    const wav = makeWav({ rate: RATE, channels: 2, samples: [Number.NaN, ...sineSamples(2, FRAMES, RATE, 1000).slice(1)] });
    const verdict = judge(corpus({ wavs: new Map([["sine", wav]]), references: { sine: renderHash(parseWav(wav)) } }));
    expect(verdict.lines).toStrictEqual(["FAIL     sine: channel 0: off the expected sine by Infinity (at most 2.98e-8)"]);
  });

  test("checks the rate, the duration and the channels against the case and the expectations", () => {
    const verdict = judge(
      corpus({
        cases: new Map([["sine", { sample_rate: RATE * 2, frames: FRAMES + 1 }]]),
        expectations: { sine: { sample_rate: RATE + 1, frames: FRAMES - 1, channels: [CHANNEL, CHANNEL, CHANNEL] } },
      }),
    );
    expect(verdict.lines).toStrictEqual([
      "FAIL     sine: sample rate (case): the WAV file has 8000, expected 16000",
      "FAIL     sine: sample rate (expectations): the WAV file has 8000, expected 8001",
      "FAIL     sine: frames (case): the WAV file has 64, expected 65",
      "FAIL     sine: frames (expectations): the WAV file has 64, expected 63",
      "FAIL     sine: channels (expectations): the WAV file has 2, expected 3",
    ]);
    expect(verdict.failures).toBe(5);
  });

  test("allows no error at all for silence", () => {
    // Amplitude 0: the bound is 0, so the smallest subnormal is already an error.
    const zero = { sine: { frequency: 1000, amplitude: 0, phase: 0 } };
    const judged = (value: number): Verdict => {
      const flat = makeWav({ rate: RATE, channels: 1, samples: Array.from({ length: FRAMES }, (_empty, index) => (index === 5 ? value : 0)) });
      return judge(
        corpus({
          wavs: new Map([["sine", flat]]),
          references: { sine: renderHash(parseWav(flat)) },
          expectations: { sine: { sample_rate: RATE, frames: FRAMES, channels: [zero] } },
        }),
      );
    };
    expect(judged(0).lines[0]).toContain("sine error 0.00e+0");
    expect(judged(2 ** -149).lines).toStrictEqual(["FAIL     sine: channel 0: off the expected sine by 1.40e-45 (at most 0.00e+0)"]);
  });

  test.each([
    ["case file", { cases: new Map() }],
    ["reference hash", { references: {} }],
    ["expectations", { expectations: {} }],
    ["WAV file", { wavs: new Map() }],
  ])("reports a missing %s alone", (piece, change) => {
    expect(judge(corpus(change))).toStrictEqual({ lines: [`FAIL     sine: missing ${piece}`], failures: 1 });
  });

  test("reports what is missing for each case, sorted by name", () => {
    const verdict = judge({
      cases: new Map([["b", {}]]),
      references: { c: HASH },
      expectations: { a: { sample_rate: RATE, frames: FRAMES, channels: [CHANNEL] } },
      wavs: new Map([["d", WAV]]),
    });
    expect(verdict).toStrictEqual({
      lines: [
        "FAIL     a: missing case file, reference hash, WAV file",
        "FAIL     b: missing reference hash, expectations, WAV file",
        "FAIL     c: missing case file, expectations, WAV file",
        "FAIL     d: missing case file, reference hash, expectations",
      ],
      failures: 4,
    });
  });

  test("reports an invalid case file or WAV file as a problem of that case", () => {
    expect(judge(corpus({ cases: new Map([["sine", {}]]) })).lines).toStrictEqual(["FAIL     sine: invalid conformance input: sine: sample_rate is not a finite number"]);
    expect(judge(corpus({ wavs: new Map([["sine", new Uint8Array(3)]]) })).lines).toStrictEqual(["FAIL     sine: invalid WAV file: not a RIFF WAVE file"]);
  });

  test("lets other errors through", () => {
    const hostile = { get sample_rate(): number {
      throw new RangeError("boom");
    } };
    expect(() => judge(corpus({ cases: new Map([["sine", hostile]]) }))).toThrow(new RangeError("boom"));
  });

  test("rejects invalid references or expectations as a whole", () => {
    expect(() => judge(corpus({ references: [] }))).toThrow("references.json is not an object");
    expect(() => judge(corpus({ expectations: [] }))).toThrow("expectations.json is not an object");
  });
});

describe("an empty corpus", () => {
  test("fails: judging nothing proves nothing", () => {
    expect(judge({ cases: new Map(), references: {}, expectations: {}, wavs: new Map() })).toStrictEqual({
      lines: ["FAIL     the corpus has no case"],
      failures: 1,
    });
  });
});
