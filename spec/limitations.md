# Limitations and trade-offs

What the project does not prove, or proves only in part, and the trade-offs it accepted, in one place. Each entry says what is limited, why, what it costs, and where it is decided or tracked; the decision or the item it names is the source, this page only gathers them. An entry leaves this page when the roadmap step it names removes it.

## Bit-identical renders

- **A browser that flushes subnormals on its audio thread.** Firefox on x86_64 runs its audio thread, and the engine's WebAssembly on it, with subnormal operands and results flushed to zero, which no page can read or change; Chromium and WebKit keep IEEE arithmetic, and on aarch64 all three do (measured; [`engine-design.md`](engine-design.md), decision 11). Where the engine flushes, no bit changes, but for the values a binary64 render given as binary32 rounds up to 2^-126, which a flush mode on aarch64 gives as zero instead (decision 11; no browser measured on aarch64 flushes); elsewhere, a live render in that browser equals the native one only as long as no subnormal arises outside the engine's flush points: true of every case the browser tests play live, not proven for every patch. The difference is far below anything audible. The engine reports the arithmetic it observes when it starts. Accepted: avoiding it would cost every user latency or speed, for one browser on one architecture.
- **The bits of a NaN.** A sum or a product whose result is NaN gives a NaN whose payload and sign may differ between targets and builds ([`engine-design.md`](engine-design.md), decision 28; #24 step 2 decides how sums treat NaN).

## Output stage

- **True peak above the ceiling.** Every sample the protected stage gives is at or below the ceiling, proven; its true peak (between samples) can exceed it by the detector's under-read, bounded only for an in-band sine whose period fits the window the limiter holds its needs over ([`engine-design.md`](engine-design.md), decision 12; [`measurements.md`](measurements.md), "The true-peak limiter"). For other signals the overshoot is not established yet: on noise, measured against a 16-times oversampled reference, it was found up to 2.1, 1.0 and 0.4 dB with the standard, precise and mastering detectors, and up to 0.23, 0.14 and 0.05 dB for noise below 0.4 of the rate, figures of one search, not bounds (#24 step 118).
- **Below the window.** No true-peak bound is derived for a sine whose period is longer than that window (below about 290 to 330 Hz at the default 1.5 ms lookahead), where the gain moves within each period; the playground's tests allow a measured shortfall instead (#24 step 112).
- **In the bass.** The release is meant neither to pump nor to distort, but pumping is measured nowhere, and a sine 6 dB over the ceiling was found with 3.7% THD+N at 50 Hz with the default settings; a hold separate from the lookahead is decided against it, and the adaptivity's default and the fast release's divisor are derived after it, not yet (#24 step 114).
- **Below the ceiling by the detector's over-read.** A detector reading a peak high limits a sine below the ceiling, by its largest gain `G`: up to 1.044 in band for the precise detector, the default, about 0.37 dB (#24 step 132).
- **The mastering detector live.** Whether a live plan may use the mastering detector, the dearest, waits for the stage's measured cost (#24 step 113).
- **A lookahead set live takes up to four times the maximum lookahead.** The new window fills with its own reductions, then fades in over the longer window, and a lookahead set meanwhile waits for that change to end: up to 20 ms at the 5 ms maximum before the last one asked for is in force (decision 12). Per frame, the gain still moves by the shorter window's attack, which a burst just beyond the old window needs to be held in time.
- **Its cost.** What the limiter costs while it protects, each detector, and the 128-frame module the playground ships, are not measured by the benchmarks yet (#24 step 113).

## Visual library

- **Point sampling, no motion blur.** Each image shows one instant, so fast motion strobes and can alias in time ([`visual-design.md`](visual-design.md), decision 2); whether boxes integrate over time is decided by #35 step 2.
- **A box filter only.** A pixel is the exact area a shape covers: the standard for vector shapes, but it leaves some aliasing on fine detail, which other filters reduce (#35 step 7).
- **Subnormals and negative zero not decided.** No law refuses a subnormal value or -0 in an image, and nothing says what a target flushing subnormals does to the image hash; no box outputs either today, so no bit depends on it. Decided at the GPU step (#34 step 5; [`visual-design.md`](visual-design.md), section 9).
- **Exact, not ideal.** The square is proven equal to its exact coverage at 1/4096 of a pixel, not to the ideal, unrounded square. Derived from where each edge can fall, in subpixel units (the side is rounded down by at most 15/16; the top edge errs within [−1/2, 15/32], the bottom within [−31/32, 0], the left within (−1, 15/16), the right within (−31/16, 0]): a pixel's value differs from the ideal by less than 93/32 of 1/4096 (about 2.906/4096, the supremum), and by less than 31/16 of 1/4096 (about 1.94/4096) near black, a pixel the square barely covers. Through the sRGB transfer, near black, that is below 1.6 codes at 8 bits, 6.3 at 10 and 25.1 at 12. The derivation is not yet checked by a test, and no output transfer exists yet: #35 step 7 measures each image against the ideal shape, and whether the difference shows is a perceptual question it answers.
- **No output transfer or quantization.** Images are linear-light floats; sRGB or another display encoding, with deterministic dithering, comes with the color pipeline and the output stage (#35 steps 6 and 10).
- **Square pixels assumed.** An image has no pixel aspect ratio: the square is square on square pixels only (#35 step 2).
- **Exactness, not perception, measured.** The square's measurements are bit equalities with an exact reference ([`measurements.md`](measurements.md), Images); perceptual measurements (aliasing of moving edges, flicker, banding) come with #35 step 7, and the visual library has no benchmark yet (the first with #34 step 5, one per visual box with #35 step 12).
- **WebAssembly in Node only.** The visual library's own module is tested in Node, not in a browser; in browsers the product loads, on each thread, one module linking what that thread needs (externals apart, each its own module), which #34 step 6 builds and tests there ([`engine-design.md`](engine-design.md), decision 33).
- **Out of memory.** Creating an image cannot fail: where a machine cannot provide its memory (up to 512 MiB at the largest size), a native program aborts (`visual-cli` with no `error:` line) and the WebAssembly instance traps, lost with the image it held. A fallible creation, refused with the size named, comes with the playground (#34 step 6).
- **Little-endian hosts and regular case files assumed.** The visual module's wrapper reads image values in the host's byte order, right on every little-endian JavaScript host and wrong on a big-endian one (roadmap #24 step 103); `visual-cli` reading a case path that is not a regular file (a FIFO) waits on it instead of refusing it (#24 step 104).
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
