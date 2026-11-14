# Visual library design decisions (first image)

The decisions the first version of the visual library is built on (roadmap #34 steps 1 to 9). They are the minimum the first image needs, and **provisional**: the full design decisions of the visual engine come with roadmap #35 step 2, which confirms, extends or replaces each one before more visual boxes are written. Each one gives the decision, why, the alternatives rejected and its consequences.

The visual library mirrors the audio engine's design principles (`engine-design.md`): a contract every box implements, exact and deterministic rendering, laws checked on every box, proof across targets. It shares no code with the audio engine.

## Words

| Word | Meaning |
|---|---|
| **image** | One picture of a visual render, at one instant. Never called a frame: a frame is one instant of the audio channels (`GLOSSARY.md`) |
| **image rate** | Images per second, an exact fraction (`24000/1001`) |
| **image index** | The position of an image in a sequence, from 0 |
| **visual box** | One processing unit of a visual patch: what renders images |
| **subpixel unit** | 1/4096 of a pixel side, the unit of every coordinate |
| **coverage** | The area of a pixel a shape covers, in subpixel units of area |
| **trace** | What produced a point of a box's output: the answer by which a gesture on an output finds what it touched |

---

## 1. A standalone library

**Decision.** The visual library is usable on its own: it depends on no other part of the project, the audio engine included (its tests use the workspace's counting allocator, a test-only crate), as `spec/engine-design.md` decision 32 requires of every library. xtask's library gate checks it: the crates holding `CrateRole::VisualLibrary` depend, within the project, only on each other, and their tests also on `visual-testkit` and the counting allocator; its third-party dependencies are reviewed and, all being bit-defining, pinned exactly, as the audio library's are. Exact musical time will come from the shared time crate (#37), the one other crate it may depend on. What spans sound and image (a clock shared with the audio engine, a scene with sound sources) lives in a layer above both libraries, which uses each only through its public interface.

**Why.** Each library is released as open source, ready to use as is. A shared layer above costs nothing at run time: one program, one WebAssembly binary, the compiler sees through the crates; benchmarks are to prove it (#35 step 3).

**Rejected.** One engine for sound and image: neither could be used alone.

## 2. Exact time

**Decision.** An image rate is an exact, reduced fraction `numerator / denominator` images per second, from 1 to 1000 images per second (`ImageRate`). Image `n` is at the exact instant `n × denominator / numerator` seconds. A visual box renders an image as a pure function of its index: **random access**, any image can be rendered first, and rendering it twice gives the same bits.

**Why.** Broadcast rates are not whole numbers (`24000/1001`); a floating-point rate drifts and differs between targets. Random access is what cues, seeking and editing need; the audio engine's seek law (`engine-design.md`) is the same property.

**Rejected.** Floating-point time; a per-image accumulator, which drifts and forbids random access.

Durations (a cycle, later a cue) are exact reduced fractions of seconds too (`Seconds`).

**Consequences.** The square computes its position from the image index in 128-bit integer arithmetic: exact for every index up to 2⁶⁴ − 1. Boxes with state (feedback, particles) will need checkpoints to keep random access (#35 step 8).

## 3. Images

**Decision.** An image has a width and a height from 1 to 16 384 pixels, and its pixels row by row from the top, each row from the left, each pixel four 32-bit floats: red, green, blue, alpha. One color encoding for now: **linear light** (no transfer function), the primaries and white point of ITU-R BT.709 (those of sRGB), full range, alpha premultiplied. Every image carries its encoding. An image allocates all its memory when it is created; rendering into it never allocates.

**Why.** Blending and filtering are only correct in linear light; floating point keeps headroom and high dynamic range open; carrying the encoding on every image makes later encodings (Display P3, BT.2020, PQ, HLG) additions, not breaking changes.

**Rejected.** 8-bit sRGB images: blending in a non-linear encoding is wrong, and moving to linear later is costly.

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

A square wider than the image does not move and is clipped on the right. Its trace answers shape 0 inside the square (edges half-open, like pixels) and the background elsewhere.

**Why.** The smallest shape that tests everything: exact coverage at every subpixel position, exact time, random access. It will follow the phase of an audio oscillator through the layer above both libraries (#35 step 3).

## 7. Structure and laws, as for sound

**Decision.** The visual library is split like the audio engine: `visual-core` (the contract, images, exact time, the reference renderer), `visual-boxes` (the boxes), `visual-hash` (the image hash) and `visual-testkit` (the laws). Every visual box passes the **visual box laws** of `visual-testkit` at every image rate, size and index of the shared visual test matrix (`conformance/visual/test-matrix.json`): determinism (two fresh instances give the same bits), random access (an instance that rendered other images first, in any order, gives the same bits), overwrite (rendering overwrites every value), finite values (no NaN or infinity, alpha within [0, 1]), no heap operation while rendering, and a pure trace. Each law is proven to catch a box that breaks it.

**Why.** A new box cannot escape the laws, and a law that cannot fail proves nothing.

## 8. The exact CPU reference renderer

**Decision.** The visual library's core is `no_std` and forbids `unsafe`, like the audio engine's: it cannot reach the OS, the clock, threads or the GPU, and it builds for every portability target (`cargo xtask check`), a microcontroller included. It renders sequences of images exactly (`render_sequence`); every other renderer, the GPU one included, is measured against it.

**Why.** The proof tiers of visuals: the CPU reference and the logic are exact, the same bits on every target; floating-point GPU rendering is measured within a derived threshold of the reference (#34 step 5).
