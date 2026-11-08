# Render hash, version 1

The **render hash** identifies a render bit for bit: two renders have the same hash only if they have the same sample rate, the same number of channels and the same bits in every sample. It is how conformance is proven across targets: native, WebAssembly, every browser and every SDK must reproduce the reference hashes of `conformance/references.json`.

Three implementations follow this document, independently: `render_hash` (`crates/engine-hash`), `renderHash` (`bindings/wasm/ts/hash.ts`, exported as `@poyetik/engine-wasm/hash`) and the oracle's `renderHash` (`tooling/oracle/src/hash.ts`). Each one is tested against the known-answer vectors of [`render-hash-vectors.json`](render-hash-vectors.json).

## Input

A render is:

- a **sample rate**, a whole number of hertz that fits 32 bits unsigned;
- a **channel count**;
- its **samples**, 32-bit IEEE 754 floats, **interleaved by channel**: the samples of the first frame, channel 0 first, then those of the second frame, and so on. The sample count is the number of frames times the channel count.

## Hashed bytes

The hash is the SHA-256 (FIPS 180-4) of these bytes, in this order, every number **little-endian**:

| Offset | Size (bytes) | Contents |
|---|---|---|
| 0 | 13 | The ASCII tag `render-hash/1`, without terminator |
| 13 | 4 | The sample rate, unsigned |
| 17 | 8 | The channel count, unsigned |
| 25 | 8 | The **sample count** (frames × channels), unsigned |
| 33 | 4 per sample | The bit pattern of every sample, as an unsigned 32-bit integer, in the interleaved order |

Samples are hashed by their bits, never by their value: `0` and `-0` differ, every NaN payload is kept, a subnormal is hashed as it is. No sample is clipped, rounded, sanitized or reordered before hashing.

The hash is written as 64 lowercase hexadecimal digits, the first byte of the digest first.

## Versioning

The tag names the version of the scheme. Any change to the hashed bytes (their order, a field, a width, the hash function) takes a new tag (`render-hash/2`, …), never reused, and changes every reference hash through `cargo xtask bless`, with a commit that says why. A render hash is only compared with a hash of the same version.

## Worked example

The vector `special-values` of `render-hash-vectors.json`: 11 025 Hz, 2 channels, 3 frames, so 6 samples: `0`, `-0` (frame 0), the smallest subnormal, `1` (frame 1), `-0.5` and a quiet NaN with payload 1 (frame 2).

| Bytes (hexadecimal) | Field |
|---|---|
| `72 65 6e 64 65 72 2d 68 61 73 68 2f 31` | the tag `render-hash/1` |
| `11 2b 00 00` | the rate, 11 025 |
| `02 00 00 00 00 00 00 00` | 2 channels |
| `06 00 00 00 00 00 00 00` | 6 samples |
| `00 00 00 00` | `0` (`0x00000000`) |
| `00 00 00 80` | `-0` (`0x80000000`) |
| `01 00 00 00` | the smallest subnormal (`0x00000001`) |
| `00 00 80 3f` | `1` (`0x3f800000`) |
| `00 00 00 bf` | `-0.5` (`0xbf000000`) |
| `01 00 c0 7f` | a quiet NaN with payload 1 (`0x7fc00001`) |

Its render hash is `3dab487551f6c3e1a40fa4d4823546661761b9a54dcad993aa522cf252763770`.

## Known-answer vectors

[`render-hash-vectors.json`](render-hash-vectors.json) holds the vectors every implementation must reproduce. Each one gives its `rate`, its `channels`, its `samples` as the bit patterns of the interleaved samples (8 hexadecimal digits each, so that `-0` and NaN payloads are exact), the hashed `bytes` in hexadecimal, and the expected `sha256`. They were computed by an implementation independent of the three above (Python's `hashlib` over the bytes of this document).
