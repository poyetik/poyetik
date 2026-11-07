// Builds WAV files for the tests, field by field, with any field overridable.

/** Fields of a 32-bit float WAV file, all overridable to build invalid files. */
export interface WavFields {
  readonly rate: number;
  readonly channels: number;
  /** Interleaved samples. */
  readonly samples: readonly number[];
  readonly format?: number;
  readonly bits?: number;
  readonly blockAlign?: number;
  readonly byteRate?: number;
  readonly factFrames?: number;
  readonly fmtSize?: number;
  /** Extra bytes after the data chunk. */
  readonly trailer?: readonly number[];
  /** Replaces the computed RIFF size. */
  readonly riffSize?: number;
}

/** Builds the bytes of a WAV file. */
export function makeWav(fields: WavFields): Uint8Array {
  const fmtSize = fields.fmtSize ?? 18;
  const dataSize = fields.samples.length * 4;
  const trailer = fields.trailer ?? [];
  const size = 12 + 8 + fmtSize + 12 + 8 + dataSize + trailer.length;
  const bytes = new Uint8Array(size);
  const view = new DataView(bytes.buffer);
  const ascii = (offset: number, text: string): void => {
    bytes.set(new TextEncoder().encode(text), offset);
  };
  ascii(0, "RIFF");
  view.setUint32(4, fields.riffSize ?? size - 8, true);
  ascii(8, "WAVE");
  ascii(12, "fmt ");
  view.setUint32(16, fmtSize, true);
  const blockAlign = fields.blockAlign ?? fields.channels * 4;
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
    view.setFloat32(offset + 8 + index * 4, sample, true);
  });
  bytes.set(trailer, offset + 8 + dataSize);
  return bytes;
}

/** `frames` frames of a sine, the same on every channel, interleaved. */
export function sineSamples(channels: number, frames: number, rate: number, frequency: number): number[] {
  return Array.from({ length: frames * channels }, (_, index) => Math.fround(Math.sin((2 * Math.PI * frequency * Math.floor(index / channels)) / rate)));
}
