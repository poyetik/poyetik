# Image hash, version 1

The **image hash** identifies a sequence of images bit for bit: two sequences have the same hash only if they have the same image rate, the same size, the same color encoding, the same number of images and the same bits in every value. It is how the visual library proves conformance across targets, as the render hash ([`render-hash.md`](render-hash.md)) does for sound.

`image_hash` (`crates/visual-hash`) follows this document, tested against the known-answer vectors of [`image-hash-vectors.json`](image-hash-vectors.json); every other implementation (the oracle's, the WebAssembly package's) will be tested against the same vectors.

## Input

A sequence of images is:

- an **image rate**, `numerator / denominator` images per second, both terms unsigned 32-bit integers (a reduced fraction, `spec/visual-design.md`);
- a **width** and a **height** in pixels, unsigned 32-bit integers, the same for every image;
- a **color encoding**, by its code: `1` for linear-light BT.709 RGBA in 32-bit floats with premultiplied alpha, the only one so far;
- its **images**, none or more, in order. Each image holds its values row by row from the top, each row from the left, each pixel red, green, blue, alpha: width × height × 4 values, each a 32-bit IEEE 754 float.

## Hashed bytes

The hash is the SHA-256 (FIPS 180-4) of these bytes, in this order, every number **little-endian**:

| Offset | Size (bytes) | Contents |
|---|---|---|
| 0 | 12 | The ASCII tag `image-hash/1`, without terminator |
| 12 | 4 | The rate's numerator, unsigned |
| 16 | 4 | The rate's denominator, unsigned |
| 20 | 4 | The width, unsigned |
| 24 | 4 | The height, unsigned |
| 28 | 4 | The color encoding code, unsigned |
| 32 | 4 per value | The bit pattern of every value of every image, as an unsigned 32-bit integer, image after image, in the order above |
| end − 8 | 8 | The **number of images**, unsigned |

The number of images comes last, so that a sequence can be hashed as it is rendered, without knowing its length in advance; the size is fixed by the header, so the bytes are never ambiguous.

Values are hashed by their bits, never by their value: `0` and `-0` differ, every NaN payload is kept, a subnormal is hashed as it is. No value is clipped, rounded, sanitized or reordered before hashing.

The hash is written as 64 lowercase hexadecimal digits, the first byte of the digest first.

## Versioning

The tag names the version of the scheme. Any change to the hashed bytes (their order, a field, a width, the hash function) takes a new tag (`image-hash/2`, …), never reused, and changes every reference hash, with a commit that says why. An image hash is only compared with a hash of the same version.

## Worked example

The vector `one-pixel-white` of `image-hash-vectors.json`: 30000/1001 images per second, 1 × 1 pixel, encoding 1, one image whose pixel is opaque white.

| Bytes (hexadecimal) | Field |
|---|---|
| `69 6d 61 67 65 2d 68 61 73 68 2f 31` | the tag `image-hash/1` |
| `30 75 00 00` | the numerator, 30 000 |
| `e9 03 00 00` | the denominator, 1001 |
| `01 00 00 00` | the width, 1 |
| `01 00 00 00` | the height, 1 |
| `01 00 00 00` | the encoding, 1 |
| `00 00 80 3f` × 4 | red, green, blue and alpha, each `1` (`0x3f800000`) |
| `01 00 00 00 00 00 00 00` | 1 image |

Its image hash is `14fcb3f12d773b2c2b208fe6aba27217e69e6b296ae0383e96fa611a98831dd4`.

## Known-answer vectors

[`image-hash-vectors.json`](image-hash-vectors.json) holds the vectors every implementation must reproduce. Each one gives its `rate` (numerator and denominator), `width`, `height`, `encoding`, its `images` as the bit patterns of their values (8 hexadecimal digits each, so that `-0` and NaN payloads are exact), the hashed `bytes` in hexadecimal, and the expected `sha256`. They were computed independently of every implementation (Python's `hashlib` and `struct` over the bytes of this document).
