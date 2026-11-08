import { describe, expect, test } from "vitest";
import { WavError, parseWav } from "../src/wav.ts";
import { makeWav } from "./fixtures.ts";

const VALID = { rate: 48_000, channels: 2, samples: [0, 0.5, -0.5, 1] };

describe("parseWav", () => {
  test("reads the format and the raw samples", () => {
    const wav = parseWav(makeWav(VALID));
    expect(wav).toMatchObject({ rate: 48_000, channels: 2, frames: 2 });
    const view = new DataView(wav.data.buffer, wav.data.byteOffset, wav.data.byteLength);
    expect([0, 1, 2, 3].map((index) => view.getFloat32(index * 4, true))).toStrictEqual([0, 0.5, -0.5, 1]);
    expect(wav.data.byteLength).toBe(16);
  });

  test("reads a file inside a larger buffer", () => {
    const file = makeWav(VALID);
    const padded = new Uint8Array(file.length + 3);
    padded.set(file, 3);
    expect(parseWav(padded.subarray(3)).frames).toBe(2);
  });

  test("accepts a 16-byte format chunk and skips unknown chunks, aligned on even offsets", () => {
    // A 3-byte "LIST" chunk, padded to 4, after the data.
    const trailer = [...new TextEncoder().encode("LIST"), 3, 0, 0, 0, 1, 2, 3, 0];
    expect(parseWav(makeWav({ ...VALID, fmtSize: 16, trailer })).frames).toBe(2);
  });

  test("needs its chunks after a valid 12-byte RIFF header", () => {
    const header = new Uint8Array([...new TextEncoder().encode("RIFF"), 4, 0, 0, 0, ...new TextEncoder().encode("WAVE")]);
    expect(() => parseWav(header)).toThrow(new WavError("missing fmt, fact or data chunk"));
  });

  test.each([
    ["too short", new Uint8Array(4), "not a RIFF WAVE file"],
    ["not RIFF", Uint8Array.from(makeWav(VALID), (byte, index) => (index === 0 ? 0x58 : byte)), "not a RIFF WAVE file"],
    ["not WAVE", Uint8Array.from(makeWav(VALID), (byte, index) => (index === 8 ? 0x58 : byte)), "not a RIFF WAVE file"],
    ["wrong RIFF size", makeWav({ ...VALID, riffSize: 1 }), "RIFF size does not match the file size"],
    ["truncated chunk header", makeWav({ ...VALID, trailer: [0x4c, 0x49, 0x53, 0x54] }), "truncated chunk header"],
    ["chunk past the end", makeWav({ ...VALID, trailer: [0x4c, 0x49, 0x53, 0x54, 9, 0, 0, 0] }), 'chunk "LIST" extends past the end of the file'],
    ["duplicate chunk", makeWav({ ...VALID, trailer: [0x64, 0x61, 0x74, 0x61, 0, 0, 0, 0] }), 'duplicate chunk "data"'],
    ["short fmt chunk", makeWav({ ...VALID, fmtSize: 14 }), "fmt chunk too short"],
    ["integer samples", makeWav({ ...VALID, format: 1 }), "not 32-bit float samples (format 1, 32 bits)"],
    ["64-bit samples", makeWav({ ...VALID, bits: 64 }), "not 32-bit float samples (format 3, 64 bits)"],
    ["no channel", makeWav({ rate: 48_000, channels: 0, samples: [] }), "no channel or no sample rate"],
    ["no rate", makeWav({ ...VALID, rate: 0 }), "no channel or no sample rate"],
    ["block alignment", makeWav({ ...VALID, blockAlign: 4 }), "inconsistent block alignment or byte rate"],
    ["byte rate", makeWav({ ...VALID, byteRate: 1 }), "inconsistent block alignment or byte rate"],
    ["partial frame", makeWav({ ...VALID, samples: [0, 0.5, 1] }), "data is not a whole number of frames"],
    ["fact mismatch", makeWav({ ...VALID, factFrames: 3 }), "fact chunk does not match the data"],
  ])("rejects a file with %s", (_, bytes, detail) => {
    expect(() => parseWav(bytes)).toThrow(new WavError(detail));
  });

  test("rejects a short fact chunk", () => {
    const bytes = makeWav(VALID);
    // fact chunk size field: 12 (RIFF) + 26 (fmt) + 4.
    new DataView(bytes.buffer).setUint32(42, 0, true);
    // Its 4 bytes now belong to no chunk: shift the data chunk right after it.
    const shortened = new Uint8Array([...bytes.subarray(0, 46), ...bytes.subarray(50)]);
    new DataView(shortened.buffer).setUint32(4, shortened.length - 8, true);
    expect(() => parseWav(shortened)).toThrow(new WavError("fact chunk does not match the data"));
  });

  test.each([
    ["fmt", 12],
    ["fact", 38],
    ["data", 50],
  ])("rejects a file without its %s chunk", (_, offset) => {
    const bytes = makeWav(VALID);
    bytes.set(new TextEncoder().encode("JUNK"), offset);
    expect(() => parseWav(bytes)).toThrow(new WavError("missing fmt, fact or data chunk"));
  });

  test("names its errors", () => {
    expect(new WavError("x")).toMatchObject({ name: "WavError", message: "invalid WAV file: x" });
  });
});

test("reads a file of 100 000 chunks without overflowing the stack", () => {
  // 100 000 empty chunks with distinct ids, and no "fmt " chunk: a WavError, not a RangeError.
  const count = 100_000;
  const bytes = new Uint8Array(12 + 8 * count);
  const view = new DataView(bytes.buffer);
  bytes.set([0x52, 0x49, 0x46, 0x46], 0);
  view.setUint32(4, bytes.length - 8, true);
  bytes.set([0x57, 0x41, 0x56, 0x45], 8);
  for (let index = 0; index < count; index += 1) {
    // Ids made of the index, base 26 in capital letters, after an "x": all distinct.
    bytes.set([0x78, 65 + (Math.floor(index / 676) % 26), 65 + (Math.floor(index / 26) % 26), 65 + (index % 26)], 12 + 8 * index);
  }
  expect(() => parseWav(bytes)).toThrow(WavError);
});
