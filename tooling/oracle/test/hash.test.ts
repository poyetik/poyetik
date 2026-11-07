import { expect, test } from "vitest";
import { renderHash } from "../src/hash.ts";
import { parseWav } from "../src/wav.ts";
import { makeWav } from "./fixtures.ts";

test("matches the reference vector of the Rust implementation", () => {
  // crates/engine-hash: `is_stable`.
  expect(renderHash(parseWav(makeWav({ rate: 48_000, channels: 2, samples: [0, 0.5, -0.5, 1] })))).toBe(
    "9d1756287e39b44f34500c890770292a9c31c69410b00dc87b670967cfce6397",
  );
});

test("matches an independent value for an empty render", () => {
  // Python hashlib: SHA-256 of "render-hash/1", 48000 (u32), 1 (u64), 0 (u64), little-endian.
  expect(renderHash({ rate: 48_000, channels: 1, frames: 0, data: new Uint8Array() })).toBe(
    "b39af19db79a2adf7bb979d48f7dc3d88915fbc9688f9fa86b9fb2a1930c66f8",
  );
});

test("counts samples, not frames", () => {
  const stereo = parseWav(makeWav({ rate: 48_000, channels: 2, samples: [0, 0.5, -0.5, 1] }));
  expect(renderHash({ ...stereo, frames: 1 })).not.toBe(renderHash(stereo));
  expect(renderHash({ ...stereo, channels: 4, frames: 1 })).not.toBe(renderHash(stereo));
});
