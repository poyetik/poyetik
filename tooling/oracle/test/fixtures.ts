// Builds WAV files for the tests, field by field, with any field overridable.

import { RENDER_FORMATS, type RenderFormat } from "../src/format.ts";
import { renderHash, STRICT } from "../src/hash.ts";
import { parseWav } from "../src/wav.ts";

/** Fields of a 32- or 64-bit float WAV file, all overridable to build invalid files. */
export interface WavFields {
  /** Sample rate written in the format chunk, in hertz. */
  readonly rate: number;
  /** Number of channels written in the format chunk; the samples are split among them. */
  readonly channels: number;
  /** Interleaved samples. */
  readonly samples: readonly number[];
  /** Format tag; 3 (IEEE float) by default. */
  readonly format?: number;
  /** Bits per sample; 32 by default. The samples are written as binary64 when it is 64, else as binary32. */
  readonly bits?: number;
  /** Bytes per frame; computed from the channels by default. */
  readonly blockAlign?: number;
  /** Bytes per second; computed from the rate and the block size by default. */
  readonly byteRate?: number;
  /** Frame count of the `fact` chunk; the real count by default. */
  readonly factFrames?: number;
  /** Declared size of the format chunk; 18 by default. */
  readonly fmtSize?: number;
  /** Extra bytes after the data chunk. */
  readonly trailer?: readonly number[];
  /** Replaces the computed RIFF size. */
  readonly riffSize?: number;
}

/** Builds the bytes of a WAV file. */
export function makeWav(fields: WavFields): Uint8Array {
  const fmtSize = fields.fmtSize ?? 18;
  const bytesPerSample = fields.bits === 64 ? 8 : 4;
  const dataSize = fields.samples.length * bytesPerSample;
  const trailer = fields.trailer ?? [];
  const size = 12 + 8 + fmtSize + 12 + 8 + dataSize + trailer.length;
  const bytes = new Uint8Array(size);
  const view = new DataView(bytes.buffer);
  const ascii = (at: number, text: string): void => {
    bytes.set(new TextEncoder().encode(text), at);
  };
  ascii(0, "RIFF");
  view.setUint32(4, fields.riffSize ?? size - 8, true);
  ascii(8, "WAVE");
  ascii(12, "fmt ");
  view.setUint32(16, fmtSize, true);
  const blockAlign = fields.blockAlign ?? fields.channels * bytesPerSample;
  if (fmtSize >= 16) {
    view.setUint16(20, fields.format ?? 3, true);
    view.setUint16(22, fields.channels, true);
    view.setUint32(24, fields.rate, true);
    view.setUint32(28, fields.byteRate ?? fields.rate * blockAlign, true);
    view.setUint16(32, blockAlign, true);
    view.setUint16(34, fields.bits ?? 32, true);
  }
  let offset = 20 + fmtSize;
  ascii(offset, "fact");
  view.setUint32(offset + 4, 4, true);
  view.setUint32(offset + 8, fields.factFrames ?? (fields.channels === 0 ? 0 : fields.samples.length / fields.channels), true);
  offset += 12;
  ascii(offset, "data");
  view.setUint32(offset + 4, dataSize, true);
  fields.samples.forEach((sample, index) => {
    if (bytesPerSample === 8) {
      view.setFloat64(offset + 8 + index * 8, sample, true);
    } else {
      view.setFloat32(offset + 8 + index * 4, sample, true);
    }
  });
  bytes.set(trailer, offset + 8 + dataSize);
  return bytes;
}

/** `frames` frames of a sine, the same on every channel, interleaved. */
export function sineSamples(channels: number, frames: number, rate: number, frequency: number): number[] {
  return Array.from({ length: frames * channels }, (_empty, index) => Math.fround(Math.sin((2 * Math.PI * frequency * Math.floor(index / channels)) / rate)));
}

/** `frames` frames of a sine in binary64, the same on every channel, interleaved. */
export function sineSamples64(channels: number, frames: number, rate: number, frequency: number): number[] {
  return Array.from({ length: frames * channels }, (_empty, index) => Math.sin((2 * Math.PI * frequency * Math.floor(index / channels)) / rate));
}

/** The three renders of a sine case, one WAV file per render format, and their hashes. */
export interface Renders {
  /** The WAV file of each render format. */
  readonly wavs: ReadonlyMap<RenderFormat, Uint8Array>;
  /** The reference hashes, as `references.json` holds them for one case. */
  readonly hashes: Readonly<Record<RenderFormat, string>>;
}

/**
 * The renders of a sine on `channels` channels, as the engine gives them: binary64 samples
 * for `binary64`, and the same sine rounded to binary32 for both binary32 formats (within
 * the bound of each).
 */
export function renders(channels: number, frames: number, rate: number, frequency: number): Renders {
  const wavs = new Map<RenderFormat, Uint8Array>(
    RENDER_FORMATS.map((format) => [
      format,
      format === "binary64"
        ? makeWav({ rate, channels, samples: sineSamples64(channels, frames, rate, frequency), bits: 64 })
        : makeWav({ rate, channels, samples: sineSamples(channels, frames, rate, frequency) }),
    ]),
  );
  const hash = (format: RenderFormat): string => {
    const bytes = wavs.get(format);
    if (bytes === undefined) {
      throw new Error(`no ${format} render`);
    }
    return renderHash(parseWav(bytes), format, STRICT);
  };
  return {
    wavs,
    hashes: { binary64: hash("binary64"), "binary64-as-binary32": hash("binary64-as-binary32"), binary32: hash("binary32") },
  };
}

/**
 * `make`, called once, on first use: a fixture computed at a test file's top level would run
 * while the module loads, before any test, so Stryker could not tell which tests use the code
 * it calls and would run every test, reloading the modules, for each of its mutants.
 */
export function once<T>(make: () => T): () => T {
  let made: { readonly value: T } | undefined;
  return () => {
    made ??= { value: make() };
    return made.value;
  };
}
