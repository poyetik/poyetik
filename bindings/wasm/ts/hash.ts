/**
 * The canonical render hash (`render-hash/1`), computed in JavaScript: an implementation
 * independent of the Rust one (`crates/engine-hash`), giving the same hash for the same
 * bits.
 */

/** Tag opening the hashed bytes: the version of the scheme. */
const TAG = "render-hash/1";

/** Interleaves channels (all of the same length) into one array, frame after frame, bit for bit. */
export function interleave(channels: readonly Float32Array[]): Float32Array {
  const count = channels.length;
  const out = new Float32Array((channels[0]?.length ?? 0) * count);
  // Copied as bit patterns, never as numbers: a number conversion may change a NaN.
  const bits = new Uint32Array(out.buffer);
  channels.forEach((channel, index) => {
    // Samples beyond the length of the first channel fall outside `out` and are ignored.
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
 */
export async function renderHash(rate: number, channels: number, samples: Float32Array): Promise<string> {
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
