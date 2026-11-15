# Limitations and trade-offs

What the project does not prove, or proves only in part, and the trade-offs it accepted, in one place. Each entry says what is limited, why, what it costs, and where it is decided or tracked; the decision or the item it names is the source, this page only gathers them. An entry leaves this page when the roadmap step it names removes it.

## Bit-identical renders

- **A browser that flushes subnormals on its audio thread.** Firefox on x86_64 runs its audio thread, and the engine's WebAssembly on it, with subnormal operands and results flushed to zero, which no page can read or change; Chromium and WebKit keep IEEE arithmetic, and on aarch64 all three do (measured; [`engine-design.md`](engine-design.md), decision 11). Where the engine flushes, no bit changes; elsewhere, a live render in that browser equals the native one only as long as no subnormal arises outside the engine's flush points: true of every case the browser tests play live, not proven for every patch. The difference is far below anything audible. The engine reports the arithmetic it observes when it starts. Accepted: avoiding it would cost every user latency or speed, for one browser on one architecture.
- **The bits of a NaN.** A sum or a product whose result is NaN gives a NaN whose payload and sign may differ between targets and builds ([`engine-design.md`](engine-design.md), decision 28; #24 step 2 decides how sums treat NaN).

## Visual library

- **Point sampling, no motion blur.** Each image shows one instant, so fast motion strobes and can alias in time ([`visual-design.md`](visual-design.md), decision 2); whether boxes integrate over time is decided by #35 step 2.
- **A box filter only.** A pixel is the exact area a shape covers: the standard for vector shapes, but it leaves some aliasing on fine detail, which other filters reduce (#35 step 7).
- **Subnormals and negative zero not decided.** No law refuses a subnormal value or -0 in an image, and nothing says what a target flushing subnormals does to the image hash; no box outputs either today, so no bit depends on it. Decided at the GPU step (#34 step 5; [`visual-design.md`](visual-design.md), section 9).
- **Exact, not ideal.** The square is proven equal to its exact coverage at 1/4096 of a pixel, not measured against the ideal, unrounded square: the worst pixel differs from it by about 5.3 × 10⁻⁴ (2.16/4096), invisible at 8 bits per channel but about 3 codes at 10 bits and 13 at 12 bits near black, after the output transfer. Measured against the ideal by #35 step 7.
- **No output transfer or quantization.** Images are linear-light floats; sRGB or another display encoding, with deterministic dithering, comes with the color pipeline and the output stage (#35 steps 6 and 10).
- **Square pixels assumed.** An image has no pixel aspect ratio: the square is square on square pixels only (#35 step 2).
- **Exactness, not perception, measured.** The square's measurements are bit equalities with an exact reference ([`measurements.md`](measurements.md), Images); perceptual measurements (aliasing of moving edges, flicker, banding) come with #35 step 7, and the visual boxes have no benchmark yet (#35 step 12).
- **WebAssembly in Node only.** The visual library's own module is tested in Node, not in a browser; in browsers the product loads, on each thread, one module linking what that thread needs, which #34 step 6 builds and tests there ([`engine-design.md`](engine-design.md), decision 33).
- **Out of memory.** Creating an image cannot fail: where a machine cannot provide its memory (up to 512 MiB at the largest size), a native program aborts (`visual-cli` with no `error:` line) and the WebAssembly instance traps, lost with the image it held. A fallible creation, refused with the size named, comes with the playground (#34 step 6).
- **No algorithm versions.** A visual reference is changed only through `visual-cli bless`, in a commit that says why; nothing yet refuses a changed bit at an unchanged version ([`visual-design.md`](visual-design.md), section 9; #35 step 2).

## Real time

- **Constructing the engine processor** compiles the module, instantiates it and builds the plan on the audio thread, once, before it renders (measured; to be bounded by #12 step 30).
- **The tap processor** copies and posts every render quantum while the playground's oscilloscope runs (until #12 step 21).
- **Allocation in the browser** is measured in V8 only; SpiderMonkey and JavaScriptCore are not measured yet (#12 step 21). The known deviations of the measured path are listed with the allocation test (`bindings/audio-wasm`, `pnpm test:allocation`).

## What CI runs on

- **One CI platform.** CI runs on x86_64 Linux. On aarch64 (macOS on Apple silicon), native conformance and the native floating-point mode are proven only by the pre-push check of the maintainer's Mac, until a macOS job (#20 step 1).
- **Browsers are Playwright's builds.** "WebKit" is Playwright's WebKit, not Safari, and "mobile WebKit" is the same engine at a phone's size, not iOS Safari; real phones are tested by #18 step 5.
- **No sound card.** In CI, live playback is proven to reach an audio output, never a device.

## Proofs

- **The default check proves the changes only.** It mutates the changed code; only the full check (`cargo xtask check --full`, CI) proves the rest. A green default check does not imply a green full check.
- **Branch coverage merges generic instantiations.** A branch taken by one precision only counts as covered for both (#24 step 87).
- **Review only.** What no tool can check is reviewed instead, and each case says so where it is stated: the files a doc test loads, the test data paths computed at run time, sums written inside box code, and the provenance of npm packages, which is not checked yet.
