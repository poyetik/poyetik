/**
 * The canonical render hash (`render-hash/1`, specified in `spec/render-hash.md`),
 * computed in JavaScript: an implementation independent of the Rust one
 * (`crates/engine-hash`), giving the same hash for the same bits.
 */

/** Tag opening the hashed bytes: the version of the scheme. */
const TAG = "render-hash/1";

/**
 * Interleaves channels into one array, frame after frame, bit for bit.
 * @throws {RangeError} when the channels differ in length: a hash of padded or cut
 * channels would prove nothing about the render.
 */
export function interleave(channels: readonly Float32Array[]): Float32Array {
  const count = channels.length;
  const frames = channels[0]?.length ?? 0;
  for (const channel of channels) {
    if (channel.length !== frames) {
      throw new RangeError(`every channel must have the same length: ${String(channel.length)} instead of ${String(frames)}`);
    }
  }
  const out = new Float32Array(frames * count);
  // Copied as bit patterns, never as numbers: a number conversion may change a NaN.
  const bits = new Uint32Array(out.buffer);
  channels.forEach((channel, index) => {
    new Uint32Array(channel.buffer, channel.byteOffset, channel.length).forEach((word, frame) => {
      bits[frame * count + index] = word;
    });
  });
  return out;
}

/**
 * SHA-256 of the tag, `rate` (u32), `channels` (u64), the sample count (u64) and the bits
 * of every interleaved sample (u32), all little-endian; as 64 lowercase hex digits.
 *
 * Samples are read as bits, never as numbers, so that no value (not even a NaN) changes.
 * Rejects with a `RangeError`, rather than hashing something else than what was asked,
 * when `rate` is not a 32-bit unsigned integer (any such rate: the scheme does not
 * restrict it to the rates the engine runs at), when `channels` is not a whole number
 * from 0 to 2^53 - 1, or when `samples` are not whole frames of `channels` channels (no
 * sample at all without a channel).
 */
export async function renderHash(rate: number, channels: number, samples: Float32Array): Promise<string> {
  if (!Number.isInteger(rate) || rate < 0 || rate > 0xff_ff_ff_ff) {
    throw new RangeError(`the sample rate must be a whole number from 0 to 4294967295, not ${String(rate)}`);
  }
  if (!Number.isSafeInteger(channels) || channels < 0) {
    throw new RangeError(`the channel count must be a whole number from 0 to 2^53 - 1, not ${String(channels)}`);
  }
  if (channels === 0 ? samples.length > 0 : samples.length % channels !== 0) {
    throw new RangeError(`${String(samples.length)} samples are not whole frames of ${String(channels)} channels`);
  }
  const tag = new TextEncoder().encode(TAG);
  const bits = new Uint32Array(samples.buffer, samples.byteOffset, samples.length);
  const bytes = new ArrayBuffer(tag.length + 4 + 8 + 8 + 4 * bits.length);
  const view = new DataView(bytes);
  new Uint8Array(bytes).set(tag);
  let offset = tag.length;
  view.setUint32(offset, rate, true);
  offset += 4;
  view.setBigUint64(offset, BigInt(channels), true);
  offset += 8;
  view.setBigUint64(offset, BigInt(bits.length), true);
  offset += 8;
  for (const word of bits) {
    view.setUint32(offset, word, true);
    offset += 4;
  }
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  return Array.from(digest, (byte) => byte.toString(16).padStart(2, "0")).join("");
}
