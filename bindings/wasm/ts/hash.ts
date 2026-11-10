/**
 * The canonical render hash (`render-hash/2`, specified in `spec/render-hash.md`),
 * computed in JavaScript: an implementation independent of the Rust one
 * (`crates/engine-hash`), giving the same hash for the same bits.
 */

import type { RenderFormat, Samples } from "./engine.ts";

/** Tag opening the hashed bytes: the version of the scheme. */
const TAG = "render-hash/2";

/**
 * The strict arithmetic mode, every option off: the only mode the engine's WebAssembly
 * module is built in (a build in any other is refused at compile time by `engine-wasm`),
 * so the mode its renders are hashed in.
 */
export const STRICT = 0n;

/** The flags of every arithmetic option the scheme defines: fused multiply-add, reassociable operations, platform math functions. */
const KNOWN_OPTIONS = 0b111n;

/** The largest arithmetic mode: 64 bits of flags. */
const LARGEST_MODE = 2n ** 64n - 1n;

/** The processing precision and the sample width of each render format, in bits. */
const WIDTHS: Readonly<Record<RenderFormat, { readonly precision: number; readonly samples: number }>> = {
  binary64: { precision: 64, samples: 64 },
  "binary64-as-binary32": { precision: 64, samples: 32 },
  binary32: { precision: 32, samples: 32 },
};

/**
 * Interleaves channels whose samples are `width` bytes, frame after frame, bit for bit.
 * @throws {RangeError} when the channels differ in length.
 */
function interleaved(channels: readonly Samples[], width: number): ArrayBuffer {
  const count = channels.length;
  const frames = channels[0]?.length ?? 0;
  for (const channel of channels) {
    if (channel.length !== frames) {
      throw new RangeError(`every channel must have the same length: ${String(channel.length)} instead of ${String(frames)}`);
    }
  }
  const out = new ArrayBuffer(frames * count * width);
  // Copied as bytes, never as numbers: a number conversion may change a NaN.
  const bytes = new Uint8Array(out);
  channels.forEach((channel, index) => {
    const source = new Uint8Array(channel.buffer, channel.byteOffset, channel.byteLength);
    for (let frame = 0; frame !== frames; frame += 1) {
      bytes.set(source.subarray(frame * width, (frame + 1) * width), (frame * count + index) * width);
    }
  });
  return out;
}

/**
 * Interleaves binary32 channels into one array, frame after frame, bit for bit.
 * @throws {RangeError} when the channels differ in length: a hash of padded or cut
 * channels would prove nothing about the render.
 */
export function interleave(channels: readonly Float32Array[]): Float32Array {
  return new Float32Array(interleaved(channels, Float32Array.BYTES_PER_ELEMENT));
}

/**
 * Interleaves binary64 channels into one array, frame after frame, bit for bit.
 * @throws {RangeError} when the channels differ in length.
 */
export function interleave64(channels: readonly Float64Array[]): Float64Array {
  return new Float64Array(interleaved(channels, Float64Array.BYTES_PER_ELEMENT));
}

/**
 * SHA-256 of the tag, the processing precision and the sample width (one byte each, in
 * bits), the arithmetic mode `mode` (u64 flags), `rate` (u32), `channels` (u64), the
 * sample count (u64) and the bits of every interleaved sample (u32 or u64), all
 * little-endian; as 64 lowercase hex digits.
 *
 * Samples are read as bits, never as numbers, so that no value (not even a NaN) changes.
 * The mode has no default: it is a fact the hash records, stated by every caller (the
 * module's renders are {@link STRICT}). Rejects with a `RangeError`, rather than hashing
 * something else than what was asked, when `rate` is not a 32-bit unsigned integer (any
 * such rate: the scheme does not restrict it to the rates the engine runs at), when
 * `channels` is not a whole number from 0 to 2^53 - 1, when `samples` are not whole
 * frames of `channels` channels (no sample at all without a channel), when `format` is
 * not a render format (from plain JavaScript), when their type is not the one of `format`
 * (`Float64Array` for binary64, `Float32Array` otherwise), or when `mode` is not 64 bits
 * of flags or holds a flag no arithmetic option defines.
 */
export async function renderHash(rate: number, channels: number, samples: Samples, format: RenderFormat, mode: bigint): Promise<string> {
  if (!Number.isInteger(rate) || rate < 0 || rate > 0xff_ff_ff_ff) {
    throw new RangeError(`the sample rate must be a whole number from 0 to 4294967295, not ${String(rate)}`);
  }
  if (!Number.isSafeInteger(channels) || channels < 0) {
    throw new RangeError(`the channel count must be a whole number from 0 to 2^53 - 1, not ${String(channels)}`);
  }
  if (channels === 0 ? samples.length > 0 : samples.length % channels !== 0) {
    throw new RangeError(`${String(samples.length)} samples are not whole frames of ${String(channels)} channels`);
  }
  if (!Object.hasOwn(WIDTHS, format)) {
    throw new RangeError(`unknown render format: ${format}`);
  }
  const widths = WIDTHS[format];
  if (samples.BYTES_PER_ELEMENT * 8 !== widths.samples) {
    throw new RangeError(`${format} samples are ${String(widths.samples)}-bit, not ${String(samples.BYTES_PER_ELEMENT * 8)}-bit`);
  }
  if (mode < 0n || mode > LARGEST_MODE) {
    throw new RangeError(`the arithmetic mode must be 64 bits of flags, not ${mode.toString()}`);
  }
  if ((mode & ~KNOWN_OPTIONS) !== 0n) {
    throw new RangeError(`unknown arithmetic options: 0x${(mode & ~KNOWN_OPTIONS).toString(16).padStart(16, "0")}`);
  }
  const tag = new TextEncoder().encode(TAG);
  const header = tag.length + 1 + 1 + 8 + 4 + 8 + 8;
  const bytes = new ArrayBuffer(header + samples.byteLength);
  const view = new DataView(bytes);
  new Uint8Array(bytes).set(tag);
  let offset = tag.length;
  view.setUint8(offset, widths.precision);
  view.setUint8(offset + 1, widths.samples);
  offset += 2;
  view.setBigUint64(offset, mode, true);
  offset += 8;
  view.setUint32(offset, rate, true);
  offset += 4;
  view.setBigUint64(offset, BigInt(channels), true);
  offset += 8;
  view.setBigUint64(offset, BigInt(samples.length), true);
  offset += 8;
  // The samples by their bits, little-endian: read as bytes, in the byte order of the
  // platform, then written word by word.
  const words = samples.BYTES_PER_ELEMENT === 8 ? new BigUint64Array(samples.buffer, samples.byteOffset, samples.length) : new Uint32Array(samples.buffer, samples.byteOffset, samples.length);
  if (words instanceof BigUint64Array) {
    for (const word of words) {
      view.setBigUint64(offset, word, true);
      offset += 8;
    }
  } else {
    for (const word of words) {
      view.setUint32(offset, word, true);
      offset += 4;
    }
  }
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  return Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join("");
}
