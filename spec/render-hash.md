# Render hash, version 2

The **render hash** identifies a render bit for bit: two renders have the same hash only if they have the same render format, the same arithmetic mode, the same sample rate, the same number of channels and the same bits in every sample. It is how conformance is proven across targets: native, WebAssembly, every browser and every SDK must reproduce the reference hashes of `conformance/references.json`.

Three implementations follow this document, independently: `render_hash` (`crates/audio-hash`), `renderHash` (`bindings/audio-wasm/ts/hash.ts`, exported as `@poyetik/audio-wasm/hash`) and the oracle's `renderHash` (`tooling/oracle/src/hash.ts`). Each one is tested against the known-answer vectors of [`render-hash-vectors.json`](render-hash-vectors.json).

## Input

A render is:

- its **render format** ([`engine-design.md`](engine-design.md), decision 10): the **precision** that computed it and the format of its samples, one of
  - `binary64`: computed in IEEE 754 binary64, its samples binary64;
  - `binary64-as-binary32`: computed in binary64, each sample then rounded to the nearest binary32, as a host that takes binary32 is given it (the browser's audio thread);
  - `binary32`: computed in IEEE 754 binary32, its samples binary32;

  a render computed in binary32 is never given as binary64, so no other pair exists;
- its **arithmetic mode**: the set of arithmetic options the math layer it ran on was built with (below);
- a **sample rate**, a whole number of hertz that fits 32 bits unsigned;
- a **channel count**;
- its **samples**, **interleaved by channel**: the samples of the first frame, channel 0 first, then those of the second frame, and so on. The sample count is the number of frames times the channel count.

Samples that are **not whole frames** of the channel count (a sample count that is not a multiple of it, or any sample at all without a channel) are not a render: every implementation **refuses** them, with the reason `<n> samples are not whole frames of <c> channels`, and never hashes them. No channel and no sample is a render (of a patch without output).

## Arithmetic mode

A set of **arithmetic options**, one flag each, as an unsigned 64-bit integer. Version 1 of the set defines three flags:

| Flag | Bit | Option |
|---|---|---|
| `0x1` | 0 | fused multiply-add |
| `0x2` | 1 | reassociable operations |
| `0x4` | 2 | platform math functions |

Every other bit is **refused** by every reader and every implementation: a hash is never computed for, and a mode never read with, a flag it does not know. All flags off, `0`, is the **strict** mode: IEEE 754 operations each correctly rounded, no fused multiply-add, the pinned pure-Rust `libm`. It is the only mode the engine builds today, and every reference hash is of a strict render. A later version of the set adds flags, never changes one; a mode with only known flags keeps its hash, so adding an option never forces a new reference for an existing render. [`engine-design.md`](engine-design.md), decision 10, gives the rules of each option.

## Hashed bytes

The hash is the SHA-256 (FIPS 180-4) of these bytes, in this order, every number **little-endian**:

| Offset | Size (bytes) | Contents |
|---|---|---|
| 0 | 13 | The ASCII tag `render-hash/2`, without terminator |
| 13 | 1 | The precision, in bits: 64 (`binary64`, `binary64-as-binary32`) or 32 (`binary32`) |
| 14 | 1 | The width of a hashed sample, in bits: 64 (`binary64`) or 32 (`binary64-as-binary32`, `binary32`) |
| 15 | 8 | The arithmetic mode, unsigned |
| 23 | 4 | The sample rate, unsigned |
| 27 | 8 | The channel count, unsigned |
| 35 | 8 | The **sample count** (frames × channels), unsigned |
| 43 | 8 or 4 per sample | The bit pattern of every sample, as an unsigned integer of the sample width, in the interleaved order |

Samples are hashed by their bits, never by their value: `0` and `-0` differ, every NaN payload is kept, a subnormal is hashed as it is (a render delivers none: delivery flushes them, `engine-design.md` decision 11, but the hash takes any bits). No sample is clipped, rounded, sanitized or reordered before hashing; a `binary64-as-binary32` render is hashed after its rounding to binary32, which is part of the render.

The hash is written as 64 lowercase hexadecimal digits, the first byte of the digest first.

## Versioning

The tag names the version of the scheme. Any change to the hashed bytes (their order, a field, a width, the hash function) takes a new tag (`render-hash/3`, …), never reused, and changes every reference hash through `cargo xtask bless`, with a commit that says why. A render hash is only compared with a hash of the same version. A new arithmetic option is not such a change: it is a new flag of the mode.

Version 1 (`render-hash/1`) hashed binary32 samples without precision, sample width or arithmetic mode; every reference was blessed again under version 2, and nothing reads version 1 any more.

## Worked example

The vector `special-values-binary64` of `render-hash-vectors.json`: binary64, the strict mode, 11 025 Hz, 2 channels, 3 frames, so 6 samples: `0`, `-0` (frame 0), the smallest binary64 subnormal, `1` (frame 1), `-0.5` and a quiet NaN with payload 1 (frame 2).

| Bytes (hexadecimal) | Field |
|---|---|
| `72 65 6e 64 65 72 2d 68 61 73 68 2f 32` | the tag `render-hash/2` |
| `40` | precision: 64 bits |
| `40` | sample width: 64 bits |
| `00 00 00 00 00 00 00 00` | the strict arithmetic mode |
| `11 2b 00 00` | the rate, 11 025 |
| `02 00 00 00 00 00 00 00` | 2 channels |
| `06 00 00 00 00 00 00 00` | 6 samples |
| `00 00 00 00 00 00 00 00` | `0` (`0x0000000000000000`) |
| `00 00 00 00 00 00 00 80` | `-0` (`0x8000000000000000`) |
| `01 00 00 00 00 00 00 00` | the smallest subnormal (`0x0000000000000001`) |
| `00 00 00 00 00 00 f0 3f` | `1` (`0x3ff0000000000000`) |
| `00 00 00 00 00 00 e0 bf` | `-0.5` (`0xbfe0000000000000`) |
| `01 00 00 00 00 00 f8 7f` | a quiet NaN with payload 1 (`0x7ff8000000000001`) |

Its render hash is `f60e833ff80a7c2a74e91faa059bc86209e27acaa49db2cd394ce53ce4086658`.

The same values in binary32 hash differently as `binary64-as-binary32` and as `binary32` (the precision byte alone differs), and the `binary32` render hashes differently again with the fused multiply-add flag set (the mode alone differs): the vectors `special-values-binary64-as-binary32`, `special-values-binary32` and `special-values-fused-multiply-add`.

## Known-answer vectors

[`render-hash-vectors.json`](render-hash-vectors.json) holds the vectors every implementation must reproduce. Each one gives its `format`, its `arithmetic_mode` (16 hexadecimal digits), its `rate`, its `channels`, its `samples` as the bit patterns of the interleaved samples (16 hexadecimal digits each in binary64, 8 in binary32, so that `-0` and NaN payloads are exact), the hashed `bytes` in hexadecimal, and the expected `sha256`. Its `refused_modes` are arithmetic modes every implementation must refuse, each with an unknown flag, and the `unknown_flags` (16 hexadecimal digits) every implementation names in its refusal: the unknown flags alone, never the known ones beside them. Its `refused_shapes` are inputs that are not whole frames, each given as a vector without `bytes` and `sha256`, with the `reason` every implementation gives. Each implementation's tests read exactly the keys of the file, so that a new kind of vector or refusal fails every suite that does not read it yet. The vectors were computed by an implementation independent of the three above (Python's `hashlib` and `struct` over the bytes of this document).
