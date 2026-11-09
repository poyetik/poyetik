/**
 * The canonical render hash (`render-hash/2`, specified in `spec/render-hash.md`),
 * computed with Node's own SHA-256 directly on the bytes of the WAV data: they already
 * are the little-endian bits of every sample.
 */

import { createHash } from "node:crypto";
import { precisionBits, sampleBits, type RenderFormat } from "./format.ts";
import type { Wav } from "./wav.ts";

/** The strict arithmetic mode, every option off: the mode the conformance corpus is rendered in. */
export const STRICT = 0n;

/** The flags of the arithmetic options the scheme defines: fused multiply-add, reassociable operations, platform math functions. */
export const KNOWN_OPTIONS = 0b111n;

/** The largest unsigned 64-bit integer. */
const U64_MAX = 0xffff_ffff_ffff_ffffn;

/**
 * SHA-256 of the tag, the precision and the sample width (one byte each), the arithmetic
 * mode (u64), the rate (u32), the channels (u64), the sample count (u64) and the samples,
 * little-endian, in hex.
 * @throws {RangeError} When `mode` is not an unsigned 64-bit integer, holds a flag no
 * option defines, or when the samples of `wav` are not as wide as those of `format`: a
 * hash of anything else than what was asked would prove nothing.
 */
export function renderHash(wav: Wav, format: RenderFormat, mode: bigint): string {
  if (mode < 0n || mode > U64_MAX) {
    throw new RangeError(`the arithmetic mode must be an unsigned 64-bit integer, not ${String(mode)}`);
  }
  if ((mode & ~KNOWN_OPTIONS) !== 0n) {
    throw new RangeError(`unknown arithmetic options: 0x${mode.toString(16).padStart(16, "0")}`);
  }
  if (wav.bits !== sampleBits(format)) {
    throw new RangeError(`${format} samples are ${String(sampleBits(format))}-bit, not ${String(wav.bits)}-bit`);
  }
  const header = new DataView(new ArrayBuffer(30));
  header.setUint8(0, precisionBits(format));
  header.setUint8(1, sampleBits(format));
  header.setBigUint64(2, mode, true);
  header.setUint32(10, wav.rate, true);
  header.setBigUint64(14, BigInt(wav.channels), true);
  header.setBigUint64(22, BigInt(wav.frames * wav.channels), true);
  return createHash("sha256").update("render-hash/2").update(new Uint8Array(header.buffer)).update(wav.data).digest("hex");
}
