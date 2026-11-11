import { describe, expect, test } from "vitest";
import { RENDER_FORMATS, type RenderFormat } from "../src/format.ts";
import { renderHash, STRICT } from "../src/hash.ts";
import { judge, type Corpus, type Verdict } from "../src/oracle.ts";
import { parseWav } from "../src/wav.ts";
import { makeWav, renders, sineSamples } from "./fixtures.ts";

/** Sample rate of the test case, in Hz. */
const RATE = 8000;
/** Length of the test case, in frames per channel. */
const FRAMES = 64;
/** The expected signal of each of the two channels of the case. */
const CHANNEL = { sine: { frequency: 1000, amplitude: 1, phase: 0 } };
/** The exported files of the passing case, a 1 kHz sine on two channels, and their hashes. */
const RENDERS = renders(2, FRAMES, RATE, 1000);
/** The binary64-as-binary32 render of the case: a binary32 WAV file of the binary64 sine. */
const WAV = RENDERS.wavs.get("binary64-as-binary32") ?? new Uint8Array();

/** The WAV files of the case `name`, from the renders of every format, by `<name>.<format>`. */
function wavsOf(name: string, files: ReadonlyMap<RenderFormat, Uint8Array> = RENDERS.wavs): Map<string, Uint8Array> {
  return new Map([...files].map(([format, bytes]) => [`${name}.${format}`, bytes]));
}

/** A corpus with one passing case, `sine`, changed by `change`. */
function corpus(change: Partial<Corpus> = {}): Corpus {
  return {
    cases: new Map([["sine", JSON.stringify({ sample_rate: RATE, frames: FRAMES })]]),
    references: { sine: RENDERS.hashes },
    expectations: { sine: { sample_rate: RATE, frames: FRAMES, channels: [CHANNEL, CHANNEL] } },
    wavs: wavsOf("sine"),
    ...change,
  };
}

/** The corpus with the binary64-as-binary32 render replaced by `wav`, its reference the hash of `wav`. */
function withRender(wav: Uint8Array): Partial<Corpus> {
  const wavs = wavsOf("sine");
  wavs.set("sine.binary64-as-binary32", wav);
  return { wavs, references: { sine: { ...RENDERS.hashes, "binary64-as-binary32": renderHash(parseWav(wav), "binary64-as-binary32", STRICT) } } };
}

/** The lines of a verdict about the binary64-as-binary32 render only. */
function given(verdict: Verdict): string[] {
  return verdict.lines.filter((line) => line.includes("binary64-as-binary32"));
}

describe("judge", () => {
  test("passes a case whose files match their references and the expected signal, in every format", () => {
    const verdict = judge(corpus());
    expect(verdict.failures).toBe(0);
    expect(verdict.lines).toHaveLength(3);
    RENDER_FORMATS.forEach((format, index) => {
      expect(verdict.lines[index]).toMatch(
        new RegExp(
          `^ok {7}sine ${format} ${RENDERS.hashes[format]} \\(2 channel\\(s\\), sine error \\d\\.\\d{2}e-\\d+; THD\\+N -\\d+\\.\\d dB, SFDR \\d+\\.\\d dB, THD -\\d+\\.\\d dB, aliasing none, SNR \\d+\\.\\d dB\\)$`,
        ),
      );
    });
  });

  test("holds each render to the bound of its own format", () => {
    // The binary32 render of the binary64 render's place: off its binary64 bound.
    const wavs = wavsOf("sine");
    wavs.set("sine.binary64", makeWav({ rate: RATE, channels: 2, samples: sineSamples(2, FRAMES, RATE, 1000), bits: 64 }));
    const bytes = wavs.get("sine.binary64") ?? new Uint8Array();
    const verdict = judge(corpus({ wavs, references: { sine: { ...RENDERS.hashes, binary64: renderHash(parseWav(bytes), "binary64", STRICT) } } }));
    expect(verdict.lines[0]).toMatch(/^FAIL {5}sine binary64: channel 0: off the expected sine by \d\.\d{2}e-8 \(at most 9\.29e-14\)$/);
    expect(verdict.lines.filter((line) => line.startsWith("ok "))).toHaveLength(2);
  });

  test("reports the largest error of all channels, accepted up to the bound", () => {
    // Channel 1, sample 0: the ideal is 0 and the sample is 2^-25, the output-rounding
    // term of the bound: accepted, and larger than any rounding error of channel 0.
    const samples = sineSamples(2, FRAMES, RATE, 1000);
    samples[1] = 2 ** -25;
    const verdict = judge(corpus(withRender(makeWav({ rate: RATE, channels: 2, samples }))));
    expect(verdict.failures).toBe(0);
    expect(given(verdict)[0]).toContain(`sine error ${(2 ** -25).toExponential(2)}`);
  });

  test("reports a wrong hash", () => {
    const verdict = judge(corpus({ references: { sine: { ...RENDERS.hashes, binary32: "0".repeat(64) } } }));
    expect(verdict.lines[2]).toBe(`FAIL     sine binary32: hash: the WAV file hashes to ${RENDERS.hashes.binary32}, the reference is ${"0".repeat(64)}`);
    expect(verdict.failures).toBe(1);
  });

  test("reads the hash of a render in its own format", () => {
    // The binary32 renders hold the same bits, and hash differently: their precision differs.
    expect(RENDERS.hashes.binary32).not.toBe(RENDERS.hashes["binary64-as-binary32"]);
    const swapped = { ...RENDERS.hashes, binary32: RENDERS.hashes["binary64-as-binary32"] };
    expect(judge(corpus({ references: { sine: swapped } })).failures).toBe(1);
  });

  test("reports a signal off the expected sine, channel by channel", () => {
    const off = { ...CHANNEL, sine: { ...CHANNEL.sine, frequency: 1001 } };
    const verdict = judge(corpus({ expectations: { sine: { sample_rate: RATE, frames: FRAMES, channels: [CHANNEL, off] } } }));
    // Off by a hertz over 64 samples: off the sine, and its levels far out of their bounds
    // (the fundamental's fit leaves most of the signal).
    const lines = given(verdict);
    expect(lines[0]).toMatch(/^FAIL {5}sine binary64-as-binary32: channel 1: off the expected sine by \d\.\d{2}e-\d+ \(at most 2\.98e-8\)$/);
    expect(lines[1]).toMatch(/^FAIL {5}sine binary64-as-binary32: channel 1: THD\+N -\d+\.\d dB, above its bound -\d+\.\d dB$/);
    expect(verdict.lines.every((line) => line.includes(" channel 1: "))).toBe(true);
    expect(verdict.failures).toBe(verdict.lines.length);
  });

  test("fails a signal that is not finite", () => {
    const verdict = judge(corpus(withRender(makeWav({ rate: RATE, channels: 2, samples: [Number.NaN, ...sineSamples(2, FRAMES, RATE, 1000).slice(1)] }))));
    expect(verdict.lines).toStrictEqual([
      expect.stringMatching(/^ok {7}sine binary64 /),
      "FAIL     sine binary64-as-binary32: channel 0: off the expected sine by Infinity (at most 2.98e-8)",
      "FAIL     sine binary64-as-binary32: channel 0: cannot be measured: sample 0 is not finite",
      expect.stringMatching(/^ok {7}sine binary32 /),
    ]);
  });

  test("checks the rate, the duration and the channels against the case and the expectations", () => {
    const verdict = judge(
      corpus({
        cases: new Map([["sine", JSON.stringify({ sample_rate: RATE * 2, frames: FRAMES + 1 })]]),
        expectations: { sine: { sample_rate: RATE + 1, frames: FRAMES - 1, channels: [CHANNEL, CHANNEL, CHANNEL] } },
      }),
    );
    expect(given(verdict)).toStrictEqual([
      "FAIL     sine binary64-as-binary32: sample rate (case): the WAV file has 8000, expected 16000",
      "FAIL     sine binary64-as-binary32: sample rate (expectations): the WAV file has 8000, expected 8001",
      "FAIL     sine binary64-as-binary32: frames (case): the WAV file has 64, expected 65",
      "FAIL     sine binary64-as-binary32: frames (expectations): the WAV file has 64, expected 63",
      "FAIL     sine binary64-as-binary32: channels (expectations): the WAV file has 2, expected 3",
    ]);
    expect(verdict.failures).toBe(15);
  });

  test("refuses a WAV file of another sample width than its format's, without hashing it", () => {
    const wavs = wavsOf("sine");
    wavs.set("sine.binary64", WAV);
    wavs.set("sine.binary32", RENDERS.wavs.get("binary64") ?? new Uint8Array());
    expect(judge(corpus({ wavs }))).toStrictEqual({
      lines: [
        "FAIL     sine binary64: bits per sample (format): the WAV file has 32, expected 64",
        expect.stringMatching(/^ok {7}sine binary64-as-binary32 /),
        "FAIL     sine binary32: bits per sample (format): the WAV file has 64, expected 32",
      ],
      failures: 2,
    });
  });

  test("allows no error at all for silence", () => {
    // Amplitude 0: the bound is 0, so the smallest subnormal is already an error.
    const zero = { sine: { frequency: 1000, amplitude: 0, phase: 0 } };
    const judged = (value: number): Verdict => {
      const flat = makeWav({ rate: RATE, channels: 1, samples: Array.from({ length: FRAMES }, (_empty, index) => (index === 5 ? value : 0)) });
      return judge(
        corpus({
          ...withRender(flat),
          expectations: { sine: { sample_rate: RATE, frames: FRAMES, channels: [zero] } },
        }),
      );
    };
    expect(given(judged(0))[0]).toContain("sine error 0.00e+0; levels not measured: no fundamental strictly between DC and Nyquist with a margin above its error bound)");
    expect(given(judged(2 ** -149))).toStrictEqual(["FAIL     sine binary64-as-binary32: channel 0: off the expected sine by 1.40e-45 (at most 0.00e+0)"]);
  });

  test.each([
    ["case file", { cases: new Map() }],
    ["reference hashes", { references: {} }],
    ["expectations", { expectations: {} }],
  ])("reports a missing %s alone", (piece, change) => {
    expect(judge(corpus(change))).toStrictEqual({ lines: [`FAIL     sine: missing ${piece}`], failures: 1 });
  });

  test("reports the missing WAV file of each format", () => {
    const wavs = wavsOf("sine");
    wavs.delete("sine.binary32");
    expect(judge(corpus({ wavs })).lines[2]).toBe("FAIL     sine binary32: missing WAV file");
    expect(judge(corpus({ wavs: new Map() }))).toStrictEqual({
      lines: RENDER_FORMATS.map((format) => `FAIL     sine ${format}: missing WAV file`),
      failures: 3,
    });
  });

  test("reports what is missing for each case, sorted by name", () => {
    const verdict = judge({
      cases: new Map([["b", "{}"]]),
      references: { c: RENDERS.hashes },
      expectations: { a: { sample_rate: RATE, frames: FRAMES, channels: [CHANNEL] } },
      wavs: wavsOf("d"),
    });
    expect(verdict).toStrictEqual({
      lines: [
        "FAIL     a: missing case file, reference hashes",
        "FAIL     b: missing reference hashes, expectations",
        "FAIL     c: missing case file, expectations",
        "FAIL     d: missing case file, reference hashes, expectations",
      ],
      failures: 4,
    });
  });

  test("names a case by its WAV files, its name holding dots", () => {
    const verdict = judge({ cases: new Map(), references: {}, expectations: {}, wavs: wavsOf("sine-440.25-at-96k") });
    expect(verdict.lines).toStrictEqual(["FAIL     sine-440.25-at-96k: missing case file, reference hashes, expectations"]);
  });

  test("reports a WAV file named after no render format", () => {
    const wavs = wavsOf("sine");
    wavs.set("sine", WAV);
    wavs.set("sine.binary16", WAV);
    expect(judge(corpus({ wavs }))).toMatchObject({
      lines: [
        "FAIL     sine.wav: named after no render format (binary64, binary64-as-binary32, binary32)",
        "FAIL     sine.binary16.wav: named after no render format (binary64, binary64-as-binary32, binary32)",
        expect.stringMatching(/^ok /),
        expect.stringMatching(/^ok /),
        expect.stringMatching(/^ok /),
      ],
      failures: 2,
    });
    expect(judge({ cases: new Map(), references: {}, expectations: {}, wavs: new Map([["x", WAV]]) })).toStrictEqual({
      lines: ["FAIL     x.wav: named after no render format (binary64, binary64-as-binary32, binary32)"],
      failures: 1,
    });
  });

  test("reports an invalid case file or WAV file as a problem of that render", () => {
    expect(judge(corpus({ cases: new Map([["sine", "{}"]]) })).lines[0]).toBe("FAIL     sine binary64: invalid conformance input: sine: sample_rate is not a finite number");
    expect(given(judge(corpus({ ...withRender(WAV), wavs: new Map([...wavsOf("sine"), ["sine.binary64-as-binary32", new Uint8Array(3)]]) })))).toStrictEqual([
      "FAIL     sine binary64-as-binary32: invalid WAV file: not a RIFF WAVE file",
    ]);
  });

  test("lets other errors through", () => {
    /** Bytes whose length cannot be read: an error that is neither an input nor a WAV error. */
    class Hostile extends Uint8Array {
      /** Throws, as a bug would. */
      override get length(): number {
        throw new RangeError("boom");
      }
    }
    expect(() => judge(corpus({ wavs: new Map([["sine.binary64", new Hostile(WAV)]]) }))).toThrow(new RangeError("boom"));
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
