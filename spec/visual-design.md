# Visual library design decisions (first image)

The decisions the first version of the visual library is built on (roadmap #34 steps 1 to 9). They are the minimum the first image needs, and **provisional**: the full design decisions of the visual engine come with roadmap #35 step 2, which confirms, extends or replaces each one before more visual boxes are written. Each one gives the decision, why, the alternatives rejected and its consequences.

The visual library mirrors the audio library's design principles (`engine-design.md`): a contract every box implements, exact and deterministic rendering, laws checked on every box, proof across targets. It shares no code with the audio library.

## Words

The words of the visual library (image, image rate, image index, visual box, subpixel unit, coverage, image hash, visual box laws, trace) are those of `GLOSSARY.md`, section Visuals, defined there once.

---

## 1. A standalone library

**Decision.** The visual library is usable on its own: it depends on no other part of the project, the audio library included (its tests use the workspace's counting allocator, a test-only crate), as `spec/engine-design.md` decision 32 requires of every library. xtask's library gate checks it: the crates holding `CrateRole::VisualLibrary` depend, within the project, only on each other, and their tests also on `visual-testkit` and the counting allocator, which depend only on the library and on each other, so that nothing it or its tests reach, however indirectly, is outside them; its command-line tool, `visual-cli`, is held to the same; its third-party dependencies are reviewed and, all being bit-defining, pinned exactly, as the audio library's are. Exact musical time will come from the shared time crate (#37), the one other crate it may depend on. What spans sound and image (a clock shared with the audio engine, a scene with sound sources) lives in the coordinating layer above both libraries, which uses each only through its public interface.

**Why.** Each library is released as open source, ready to use as is: its own WebAssembly module (`visual-wasm`, as `audio-wasm` for the audio library) is the artifact for those who use it alone. The coordinating layer costs nothing at run time: the product (the playground, the editor, the exports) links both libraries into one program, never one module per library side by side; in the browser each thread loads one module holding what it needs (the audio worklet never loads the drawing code, since it never draws), the page's built by `bindings/web-page` with the coordinating layer's crate, `coordinator` (`engine-design.md`, decision 33, which says who builds each when), and the compiler sees through the crates; benchmarks are to prove it (#35 step 3).

**Rejected.** One engine for sound and image: neither could be used alone.

## 2. Exact time

**Decision.** An image rate is an exact, reduced fraction `numerator / denominator` images per second, from 1 to 1000 images per second (`ImageRate`). Image `n` is at the exact instant `n × denominator / numerator` seconds. A visual box renders an image as a pure function of its index (and of its parameters and output size, section 5): **random access**, any image can be rendered first, and rendering it twice gives the same bits.

**Why.** Broadcast rates are not whole numbers (`24000/1001`); a floating-point rate drifts and differs between targets. Random access is what cues, seeking and editing need; the audio library's seek law (`engine-design.md`) is the same property.

**Rejected.** Floating-point time; a per-image accumulator, which drifts and forbids random access.

Durations (a cycle, later a cue) are exact reduced fractions of seconds too (`Seconds`).

**Point sampling.** Image `n` samples what it shows at the instant `n × denominator / numerator` alone, as a shutter open for no time would: no motion blur, so a fast motion strobes and can alias in time (a wheel turning backwards; a motion whose cycle is one image never moves). Whether a box integrates over the interval to the next image instead is decided with the visual engine's design (#35 step 2); if that changes what the square renders by default, its commit blesses the visual references again with `visual-cli bless`, saying why.

**Consequences.** The square computes its position from the image index in 128-bit integer arithmetic: exact for every index up to 2⁶⁴ − 1. Boxes with state (feedback, particles) will need checkpoints to keep random access (#35 step 8).

## 3. Images

**Decision.** An image has a width and a height from 1 to 16 384 pixels, at most 2²⁵ pixels in all (8K UHD and 16 384 × 2048 fit), and its pixels row by row from the top, each row from the left, each pixel four 32-bit floats: red, green, blue, alpha. One color encoding for now: **linear light** (no transfer function), the primaries and white point of ITU-R BT.709 (those of sRGB), full range, alpha premultiplied. Every image carries its encoding. An image allocates all its memory when it is created, and its creation can fail: where the machine cannot hold it (up to 512 MiB at the largest size), it is refused with its size named (`Image::try_new`, `VisualError::OutOfMemory`), never an abort or a trap, the WebAssembly instance kept usable (proven by filling a module's memory from outside: `bindings/visual-wasm`, conformance tests); rendering into it never allocates. Every value of an image is finite, never subnormal and never -0 (a box writes 0 instead): a target that flushes subnormals to zero, as GPUs and some processors do for speed, or that signs zero differently, then cannot change an image's bits, nor its hash, and no visible value is lost, since a subnormal is below 10⁻³⁸ and the finest step of a 16-bit display above 10⁻⁵ (decided at #34 step 5; held by the finite-values law).

**Why.** The pixel limit makes every target accept the same sizes, a size every target can request: a 32-bit target, WebAssembly included, allocates at most 2³¹ − 1 bytes at once, and 2²⁵ pixels of four floats take 2²⁹ (whether a machine has that memory is another matter: a microcontroller does not), so that two images (one replacing the other) still fit in 4 GiB. Blending and filtering are only correct in linear light; floating point keeps headroom and high dynamic range open; carrying the encoding on every image makes later encodings (Display P3, BT.2020, PQ, HLG) additions to the data and hash formats, not breaking changes there (the Rust `ColorEncoding` enum stays exhaustive on purpose: a new variant breaks every match, the image hash's included, so that none can miss it).

**Rejected.** 8-bit sRGB images: blending in a non-linear encoding is wrong, and moving to linear later is costly.

Whether a color value may exceed its alpha (emission, high dynamic range) is decided with the color pipeline (#35 step 6): the laws check only that alpha is within [0, 1].

## 4. Subpixel geometry and exact coverage

**Decision.** Every coordinate is a whole number of subpixel units, 4096 per pixel side. Pixel `(i, j)` covers the half-open square `[4096 i, 4096 (i + 1)) × [4096 j, 4096 (j + 1))`. A pixel's value is the **exact** area of it a shape covers: an integer from 0 to 4096² = 2²⁴, converted to light intensity as exactly `coverage / 2²⁴`, which a 32-bit float represents without rounding.

**Why.** Antialiasing with no approximation: the reference renderer computes exactly what an ideal box filter would, in integers, so it gives the same bits on every target with no floating-point rounding at all. The one float step, `coverage / 2²⁴`, is exact and never subnormal (its smallest nonzero result, 2⁻²⁴, is a normal number), so no host floating-point mode (flushing subnormals, a rounding mode) can change a bit: `visual-cli` proves it, every visual conformance case rendering its reference hash under a flushing host, and the small ones under every host mode that lets float work run (the shared `fpenv` crate sets them). How far the rendered square is from the ideal, unrounded one is derived in `limitations.md` and measured by #35 step 7.

**Rejected.** Supersampling (an approximation of the area); floating-point edge functions (rounding differs between targets).

**Consequences.** A law every shape box will respect: the values of an image sum exactly to the area of what it draws (checked on the square).

## 5. The visual box contract

**Decision.** A visual box implements `VisualBox`:
- `render(index, output)`: renders image `index` into `output`, overwriting every value; a pure function of the index, the box's parameters and the output size; never allocates, frees, locks or blocks;
- `trace(index, point)`: optional, what produced a point of the output: nothing declared (the default), the background, or a shape numbered by the box. Exact, computed on the CPU only when a gesture asks, so it costs nothing otherwise, and needing no image: the point carries the size of its image, and is always inside it (`ImageSize::point` builds it, refusing a point outside), so no box has to answer for one.

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

**Decision.** The visual library is split like the audio library: `visual-core` (the contract, images, exact time, the reference renderer), `visual-boxes` (the boxes), `visual-hash` (the image hash), `visual-host` (the safe square a host shows, which a C interface forwards to) and `visual-testkit` (the laws). Every visual box passes the **visual box laws** of `visual-testkit` at every image rate, size and index of the shared visual test matrix (`conformance/visual/test-matrix.json`), then at 64 random cases from a fixed seed (a rate of any terms, a size up to 64 pixels a side, two to eight indices, their edges often), so that a box right only at the values of the matrix is caught: determinism (two fresh instances give the same bits), random access (an instance that rendered the other images first, in reverse order or jumping between both ends of the matrix, each twice in a row, gives the same bits), size change (an instance that rendered every other size of the matrix first gives the same bits, and traces the points of every size the same way), overwrite (rendering overwrites every value), finite values (no NaN or infinity, no subnormal value and no -0, alpha within [0, 1]), no heap operation while rendering (in every pass), and a pure trace. Each law is proven to catch a box that breaks it, and each dimension of the matrix to matter (a box broken only at its last rate, width or height is caught), as the random cases are (a box broken only at a rate, a size or an index the matrix does not hold is caught). Every box calls the laws in its own tests: a convention, reviewed, until a registry lists the visual boxes, as the audio boxes' does (#35 step 2).

**Why.** Every box is held to the same bar, and a law that cannot fail proves nothing.

## 8. The exact CPU reference renderer

**Decision.** The visual library's core is `no_std` and forbids `unsafe`, like the audio library's: it cannot reach the OS, the clock, threads or the GPU, and it builds for every portability target (`cargo xtask check`), a microcontroller included. It renders sequences of images exactly (`render_sequence`); every other renderer, the GPU one included, is measured against it.

**Why.** The proof tiers of visuals: the CPU reference and the logic are exact, the same bits on every target; a GPU renderer either gives the reference's bits or is measured within a derived threshold of it. The square's GPU renderer (`visual-gpu`, wgpu natively, one WGSL shader) gives them exactly: the CPU places the square (`Square::placement`, 32-bit edges), the shader computes each coverage in 32-bit integers and converts it without rounding, and its tests compare every value's bits with the CPU reference at every point of the visual test matrix, at random squares and in bands of any height (an image larger than the GPU's buffers is rendered band by band); CI renders them on a software GPU (lavapipe). It lives in a Cargo workspace of its own: wgpu's tree turns on features of crates the engine shares, which one workspace would unify into the engine's builds (`crates/visual-gpu/Cargo.toml`).

## 9. Display encoding

**Decision.** An image is shown through its display encoding: 8-bit sRGB, straight alpha, four bytes a pixel (red, green, blue, alpha) in the image's order, the layout of a canvas's `ImageData` and of 8-bit image files (`visual_core::DisplayImage`). Each pixel's alpha is rounded linearly to the nearest of the 256 codes `k / 255`; a pixel whose alpha rounds to 0 is transparent black; otherwise each color is taken over alpha (undoing the premultiplication) exactly, as a real quotient never rounded, encoded by the sRGB transfer function of IEC 61966-2-1 (`12.92 v` up to 0.003 130 8, `1.055 v^(1/2.4) − 0.055` above), and rounded to the nearest code, a tie to the higher; a value at or below 0 gives 0, at or above 1 gives 255. The encoding lives in the visual library, never in each page or player, and is exact: each code is found by comparing the value with 255 thresholds (`SRGB8_THRESHOLDS`, `ALPHA8_THRESHOLDS`), the smallest 32-bit float reaching each code, computed in exact rational arithmetic and proven entry by entry by the tests (the float itself, and the one below it), so that no power function, whose rounding differs between targets, is evaluated; and no division is rounded: a color reaches a threshold `t` when `t × alpha` is at most the color, a product two 32-bit floats make exactly in 64 bits (`straight8`), which a GPU, whose division WGSL lets err by 2.5 units in the last place and which has no 64-bit floats, computes in 32-bit integers to the same codes. A page reads the module's display image in place: no copy of an image per image drawn. A GPU path does the same encoding and is proven equal to it: natively through wgpu, and in browsers through WebGPU, the encoded pixels copied into a canvas configured `rgba8unorm`, opaque. A WebGPU canvas takes only opaque or premultiplied pixels, never straight alpha: an image with transparency shown on the GPU will need its encoding premultiplied for it, decided when images get transparent (the square is opaque).

**Why.** A screen, a canvas and an 8-bit file take sRGB with straight alpha, while blending and filtering need linear light (decision 3): the conversion happens once, at the very end, the same everywhere. Written once in the library, it is specified, exact and tested once, and every page, native player and export shows the same bytes for the same image.

## 10. Not yet

What the first version of the visual library does not have, each deferred to the step that brings it (`spec/limitations.md` lists what it costs meanwhile):

- **Algorithm versions and a spent record** (the visual counterpart of `engine-design.md`, decision 29): a visual box has no algorithm version, and `visual-cli bless` rewrites `conformance/visual/references.json` from the current renders. A reference changes only through that command, in a commit that says why; refusing a changed bit at an unchanged version comes with the visual engine's design (#35 step 2), before a visual patch format can save one.
- **A registry of the visual boxes**, through which every box is held to the laws, as the audio boxes' is: until then each box calls the laws in its own tests (#35 step 2).
- **Measurements beyond exactness** (aliasing of moving edges, flicker, the banding of the 8-bit display encoding; #35 step 7) and **benchmarks** per visual box (#35 step 12): today's count the square's render, its hash and its display encoding in WebAssembly, and time the GPU's render (`spec/benchmarks.md`, "Visual library").
- **No promise of a stable interface** before 1.0: the error types and enums of the visual library may gain variants (#35 step 2).
