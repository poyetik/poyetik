/**
 * Reads 32-bit floating-point WAV files, written from the specification of the format
 * (RIFF, `fmt `, `fact`, `data` chunks), with no code shared with the engine's writer.
 */

/** A decoded WAV file. */
export interface Wav {
  /** Sample rate, in hertz. */
  readonly rate: number;
  /** Number of channels, at least 1. */
  readonly channels: number;
  /** Number of frames: samples per channel, as the data chunk holds them. */
  readonly frames: number;
  /** The samples as stored: little-endian 32-bit floats, channels interleaved. */
  readonly data: Uint8Array;
}

/** Thrown when a file is not a valid 32-bit float WAV file. */
export class WavError extends Error {
  /**
   * Describes what is wrong with the file.
   * @param detail What is wrong.
   */
  constructor(detail: string) {
    super(`invalid WAV file: ${detail}`);
    this.name = "WavError";
  }
}

/** `WAVE_FORMAT_IEEE_FLOAT`. */
const IEEE_FLOAT = 3;

/** The format chunk, decoded. */
interface Format {
  /** Number of channels, as declared. */
  readonly channels: number;
  /** Sample rate, in hertz, as declared. */
  readonly rate: number;
}

/** Reads four ASCII characters. */
function tag(view: DataView, offset: number): string {
  return String.fromCharCode(view.getUint8(offset), view.getUint8(offset + 1), view.getUint8(offset + 2), view.getUint8(offset + 3));
}

/** Decodes and checks the `fmt ` chunk. */
function format(body: DataView): Format {
  if (body.byteLength < 16) {
    throw new WavError("fmt chunk too short");
  }
  const code = body.getUint16(0, true);
  const channels = body.getUint16(2, true);
  const rate = body.getUint32(4, true);
  const byteRate = body.getUint32(8, true);
  const blockAlign = body.getUint16(12, true);
  const bits = body.getUint16(14, true);
  if (code !== IEEE_FLOAT || bits !== 32) {
    throw new WavError(`not 32-bit float samples (format ${String(code)}, ${String(bits)} bits)`);
  }
  if (channels === 0 || rate === 0) {
    throw new WavError("no channel or no sample rate");
  }
  if (blockAlign !== channels * 4 || byteRate !== rate * blockAlign) {
    throw new WavError("inconsistent block alignment or byte rate");
  }
  return { channels, rate };
}

/**
 * Reads the chunks from `offset` to the end of the file into `chunks`, by id. A loop
 * whose every round moves forward by at least the 8 bytes of a chunk header: it ends,
 * and its depth never grows with the number of chunks (a recursion overflowed the stack
 * at about 10 000 chunks, an 80 KB file).
 */
function readChunks(view: DataView, start: number, chunks: Map<string, DataView>): void {
  let offset = start;
  while (offset < view.byteLength) {
    if (offset + 8 > view.byteLength) {
      throw new WavError("truncated chunk header");
    }
    const id = tag(view, offset);
    const size = view.getUint32(offset + 4, true);
    if (offset + 8 + size > view.byteLength) {
      throw new WavError(`chunk "${id}" extends past the end of the file`);
    }
    if (chunks.has(id)) {
      throw new WavError(`duplicate chunk "${id}"`);
    }
    chunks.set(id, new DataView(view.buffer, view.byteOffset + offset + 8, size));
    // Chunks are aligned on even offsets: an odd-sized chunk is followed by a pad byte,
    // the last one too.
    offset += 8 + size + (size % 2);
    if (offset > view.byteLength) {
      throw new WavError(`chunk "${id}" lacks its pad byte`);
    }
  }
}

/** Reads a WAV file of 32-bit float samples, checking every size and field it relies on. */
export function parseWav(bytes: Uint8Array): Wav {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  if (bytes.length < 12 || tag(view, 0) !== "RIFF" || tag(view, 8) !== "WAVE") {
    throw new WavError("not a RIFF WAVE file");
  }
  if (view.getUint32(4, true) !== bytes.length - 8) {
    throw new WavError("RIFF size does not match the file size");
  }
  const chunks = new Map<string, DataView>();
  readChunks(view, 12, chunks);
  const fmt = chunks.get("fmt ");
  const fact = chunks.get("fact");
  const data = chunks.get("data");
  if (fmt === undefined || fact === undefined || data === undefined) {
    throw new WavError("missing fmt, fact or data chunk");
  }
  const { channels, rate } = format(fmt);
  if (data.byteLength % (channels * 4) !== 0) {
    throw new WavError("data is not a whole number of frames");
  }
  const frames = data.byteLength / (channels * 4);
  if (fact.byteLength < 4 || fact.getUint32(0, true) !== frames) {
    throw new WavError("fact chunk does not match the data");
  }
  return { rate, channels, frames, data: new Uint8Array(data.buffer, data.byteOffset, data.byteLength) };
}
