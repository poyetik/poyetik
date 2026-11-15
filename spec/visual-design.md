# Visual library design decisions (first image)

The decisions the first version of the visual library is built on (roadmap #34 steps 1 to 9). They are the minimum the first image needs, and **provisional**: the full design decisions of the visual engine come with roadmap #35 step 2, which confirms, extends or replaces each one before more visual boxes are written. Each one gives the decision, why, the alternatives rejected and its consequences.

The visual library mirrors the audio engine's design principles (`engine-design.md`): a contract every box implements, exact and deterministic rendering, laws checked on every box, proof across targets. It shares no code with the audio engine.

## Words

The words of the visual library (image, image rate, image index, visual box, subpixel unit, coverage, image hash, visual box laws, trace) are those of `GLOSSARY.md`, section Visuals, defined there once.

---

## 1. A standalone library

**Decision.** The visual library is usable on its own: it depends on no other part of the project, the audio engine included (its tests use the workspace's counting allocator, a test-only crate), as `spec/engine-design.md` decision 32 requires of every library. xtask's library gate checks it: the crates holding `CrateRole::VisualLibrary` depend, within the project, only on each other, and their tests also on `visual-testkit` and the counting allocator, which depend only on the library and on each other, so that nothing it or its tests reach, however indirectly, is outside them; its command-line tool, `visual-cli`, is held to the same; its third-party dependencies are reviewed and, all being bit-defining, pinned exactly, as the audio library's are. Exact musical time will come from the shared time crate (#37), the one other crate it may depend on. What spans sound and image (a clock shared with the audio engine, a scene with sound sources) lives in the coordinating layer above both libraries, which uses each only through its public interface.

**Why.** Each library is released as open source, ready to use as is: its own WebAssembly module (`visual-wasm`, as `audio-wasm` for the engine) is the artifact for those who use it alone. The coordinating layer costs nothing at run time: the product (the playground, the editor, the exports) loads one WebAssembly module linking the libraries it uses, never one per library (`engine-design.md`, decision 33), built by that layer, once per scope it runs in (the audio worklet's, the page's), and the compiler sees through the crates; benchmarks are to prove it (#35 step 3).

**Rejected.** One engine for sound and image: neither could be used alone.

## 2. Exact time

**Decision.** An image rate is an exact, reduced fraction `numerator / denominator` images per second, from 1 to 1000 images per second (`ImageRate`). Image `n` is at the exact instant `n × denominator / numerator` seconds. A visual box renders an image as a pure function of its index: **random access**, any image can be rendered first, and rendering it twice gives the same bits.

**Why.** Broadcast rates are not whole numbers (`24000/1001`); a floating-point rate drifts and differs between targets. Random access is what cues, seeking and editing need; the audio engine's seek law (`engine-design.md`) is the same property.

**Rejected.** Floating-point time; a per-image accumulator, which drifts and forbids random access.

Durations (a cycle, later a cue) are exact reduced fractions of seconds too (`Seconds`).

**Point sampling.** Image `n` samples what it shows at the instant `n × denominator / numerator` alone, as a shutter open for no time would: no motion blur, so a fast motion strobes and can alias in time (a wheel turning backwards; a motion whose cycle is one image never moves). Whether a box integrates over the interval to the next image instead is decided with the visual engine's design (#35 step 2); if that changes what the square renders by default, its commit blesses the visual references again with `visual-cli bless`, saying why.

**Consequences.** The square computes its position from the image index in 128-bit integer arithmetic: exact for every index up to 2⁶⁴ − 1. Boxes with state (feedback, particles) will need checkpoints to keep random access (#35 step 8).

## 3. Images

**Decision.** An image has a width and a height from 1 to 16 384 pixels, at most 2²⁵ pixels in all (8K UHD and 16 384 × 2048 fit), and its pixels row by row from the top, each row from the left, each pixel four 32-bit floats: red, green, blue, alpha. One color encoding for now: **linear light** (no transfer function), the primaries and white point of ITU-R BT.709 (those of sRGB), full range, alpha premultiplied. Every image carries its encoding. An image allocates all its memory when it is created; rendering into it never allocates.

**Why.** The pixel limit makes every target accept the same sizes: a 32-bit target, WebAssembly included, allocates at most 2³¹ − 1 bytes at once, and 2²⁵ pixels of four floats take 2²⁹, so that two images (one replacing the other) still fit in 4 GiB. Blending and filtering are only correct in linear light; floating point keeps headroom and high dynamic range open; carrying the encoding on every image makes later encodings (Display P3, BT.2020, PQ, HLG) additions, not breaking changes.

**Rejected.** 8-bit sRGB images: blending in a non-linear encoding is wrong, and moving to linear later is costly.

Whether a color value may exceed its alpha (emission, high dynamic range) is decided with the color pipeline (#35 step 6): the laws check only that alpha is within [0, 1].

## 4. Subpixel geometry and exact coverage

**Decision.** Every coordinate is a whole number of subpixel units, 4096 per pixel side. Pixel `(i, j)` covers the half-open square `[4096 i, 4096 (i + 1)) × [4096 j, 4096 (j + 1))`. A pixel's value is the **exact** area of it a shape covers: an integer from 0 to 4096² = 2²⁴, converted to light intensity as exactly `coverage / 2²⁴`, which a 32-bit float represents without rounding.

**Why.** Antialiasing with no approximation: the reference renderer computes exactly what an ideal box filter would, in integers, so it gives the same bits on every target with no floating-point rounding at all. 1/4096 of a pixel is far below what eyes or projectors resolve.

**Rejected.** Supersampling (an approximation of the area); floating-point edge functions (rounding differs between targets).

**Consequences.** A law every shape box will respect: the values of an image sum exactly to the area of what it draws (checked on the square).

## 5. The visual box contract

**Decision.** A visual box implements `VisualBox`:
- `render(index, output)`: renders image `index` into `output`, overwriting every value; a pure function of the index, the box's parameters and the output size; never allocates, frees, locks or blocks;
- `trace(index, output, point)`: optional, what produced a point of the output: nothing declared (the default), the background, or a shape numbered by the box. Exact, computed on the CPU only when a gesture asks, so it costs nothing otherwise.

This first version covers boxes with no image input (generators). Inputs, and tracing through boxes that transform their input (mapping an output point back to input points, zero, one or several), come with #35 step 2.

**Why.** Interaction with the output (picking, touch screens, interactive pages as a build target) is decided from the first box, since adding it to every box later would be costly.

## 6. The square

**Decision.** `Square` draws a white square on opaque black. Its side is `side / 65 536` of the image height, rounded down to a subpixel unit; it is vertically centered (its top edge rounded down). Its left edge sweeps from the left edge of the image, at the start of each cycle, towards the position where its right edge meets the right edge of the image, then jumps back; a cycle lasts an exact fraction of a second. For image `n` at rate `p / q` and a cycle of `a / b` seconds, with `W` and `H` the image width and height and `s` the side, all in subpixel units:

- elapsed cycles: `n q b / (p a)`, of which only the remainder `r = n q b mod p a` matters;
- left edge: `⌊r × max(W − s, 0) / (p a)⌋`;
- top edge: `⌊(H − s) / 2⌋`.

Within a cycle the left edge advances by `⌊max(W − s, 0) × q b / (p a)⌋` subpixel units or one more from one image to the next; it never reaches `W − s`, since `r < p a`: the last image of a cycle stops short of it, and the next cycle starts again at the left edge. A square wider than the image does not move and is clipped on the right. Its trace answers shape 0 inside the square (edges half-open, like pixels) and the background elsewhere.

**Why.** The smallest shape that tests everything: exact coverage at every subpixel position, exact time, random access. It will follow the phase of an audio oscillator through the layer above both libraries (#35 step 3).

## 7. Structure and laws, as for sound

**Decision.** The visual library is split like the audio engine: `visual-core` (the contract, images, exact time, the reference renderer), `visual-boxes` (the boxes), `visual-hash` (the image hash) and `visual-testkit` (the laws). Every visual box passes the **visual box laws** of `visual-testkit` at every image rate, size and index of the shared visual test matrix (`conformance/visual/test-matrix.json`): determinism (two fresh instances give the same bits), random access (an instance that rendered the other images first, in reverse order or jumping between both ends of the matrix, each twice in a row, gives the same bits), overwrite (rendering overwrites every value), finite values (no NaN or infinity, alpha within [0, 1]), no heap operation while rendering (in every pass), and a pure trace. Each law is proven to catch a box that breaks it, and each dimension of the matrix to matter (a box broken only at its last rate, width or height is caught). Every box calls the laws in its own tests: a convention, reviewed, until a registry lists the visual boxes, as the audio boxes' does (#35 step 2).

**Why.** Every box is held to the same bar, and a law that cannot fail proves nothing.

## 8. The exact CPU reference renderer

**Decision.** The visual library's core is `no_std` and forbids `unsafe`, like the audio engine's: it cannot reach the OS, the clock, threads or the GPU, and it builds for every portability target (`cargo xtask check`), a microcontroller included. It renders sequences of images exactly (`render_sequence`); every other renderer, the GPU one included, is measured against it.

**Why.** The proof tiers of visuals: the CPU reference and the logic are exact, the same bits on every target; floating-point GPU rendering is measured within a derived threshold of the reference (#34 step 5).

## 9. Not yet

What the first version of the visual library does not have, each deferred to the step that brings it (`spec/limitations.md` lists what it costs meanwhile):

- **Algorithm versions and a spent record** (the visual counterpart of `engine-design.md`, decision 29): a visual box has no algorithm version, and `visual-cli bless` rewrites `conformance/visual/references.json` from the current renders. A reference changes only through that command, in a commit that says why; refusing a changed bit at an unchanged version comes with the visual engine's design (#35 step 2), before a visual patch format can save one.
- **A registry of the visual boxes**, through which every box is held to the laws, as the audio boxes' is: until then each box calls the laws in its own tests (#35 step 2).
- **Measurements beyond exactness** (aliasing of moving edges, flicker, banding once images are quantized; #35 step 7) and **benchmarks** of the visual boxes with thresholds (#35 step 12).
- **No promise of a stable interface** before 1.0: the error types and enums of the visual library may gain variants (#35 step 2).
