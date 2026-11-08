/**
 * The canonical render hash (`render-hash/1`, specified in `spec/render-hash.md`),
 * computed with Node's own SHA-256 directly on the bytes of the WAV data: they already
 * are the little-endian bits of every sample.
 */

import { createHash } from "node:crypto";
import type { Wav } from "./wav.ts";

/** SHA-256 of the tag, the rate (u32), the channels (u64), the sample count (u64) and the samples, little-endian, in hex. */
export function renderHash(wav: Wav): string {
  const header = new DataView(new ArrayBuffer(20));
  header.setUint32(0, wav.rate, true);
  header.setBigUint64(4, BigInt(wav.channels), true);
  header.setBigUint64(12, BigInt(wav.frames * wav.channels), true);
  return createHash("sha256").update("render-hash/1").update(new Uint8Array(header.buffer)).update(wav.data).digest("hex");
}
