# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `mutants-equivalent.txt` and a gate in `cargo xtask check`: every TypeScript mutant disabled as equivalent is listed there with its justification; a disable comment that is not listed, a stale entry, or a broader disable (a whole file, every mutator) fails the check.
- The WebAssembly module exports `engine_abi_version`: the interface identity ("M4") and version (1), checked before anything else by the JavaScript side.
- No-allocation proofs beyond single boxes: the plan laws check that rendering any random built plan allocates nothing, and the WASM engine tests check `Engine::process`, `Instance::process` and `engine_process` (at every sample rate of the matrix). The CLAUDE.md claim about an `assert_no_alloc` check, which did not exist, is corrected.
- Seven conformance cases from the test matrix: 8 kHz near and above Nyquist and above the sample rate, 44.1 kHz with a partial last block, 20 kHz at 48 kHz, 19,999 Hz at 384 kHz, and 0 Hz. The Node tests and the four browsers render the whole corpus (each at its own rate) and check it is complete; live captures cover 48 and 44.1 kHz.
- `Plan::render_into` renders the next frames into a caller buffer, without allocating; it continues exactly like `Plan::render`.
- `engine-core`: the box contract (`Object`, `ObjectSpec`), strong types (`SampleRate`, fixed 64-frame `Block`, `NodeId`) and the deterministic execution plan (validation, topological sort, cycle detection, allocation-free processing).
- `engine-objects`: the first boxes, `osc~` (sine oscillator, 440 Hz by default) and `dac~` (stereo audio output), and their registry.
- `engine-testkit`: the laws every registered box must respect (metadata, determinism, finite output, no allocation while processing), checked with random inputs from a fixed seed.
- Patch format, version 1 (`spec/patch-format.md`), read by `engine-patch`.
- `engine-core`: `Plan::render` renders frames offline.
- `engine-hash`: canonical SHA-256 hash of a render, cross-checked with an independent implementation.
- `engine-cli`: renders conformance cases to WAV files, checks and blesses reference hashes.
- First conformance case, `sine-440`, checked by `cargo xtask check`; `cargo xtask bless` is the only way to update reference hashes.
- `engine-wasm` (`bindings/wasm`): the engine as a WebAssembly module with a minimal C interface, and its TypeScript glue (`AudioWorklet` processors, main-thread API, offline rendering, live capture), proven bit-identical to the native engine in Node and in Chromium, Firefox, WebKit and mobile WebKit.
- Playground (`apps/playground`, Vite + React): a 440 Hz sine from the Rust engine with an on/off button, a live Canvas 2D oscilloscope and its measurements (frequency, peak, sample rate), tested on Chromium, Firefox, WebKit and mobile WebKit, with accessibility checks.
- `engine-wasm`: `tap` streams the output of a node to the main thread; `capture` is built on it.
- Independent conformance oracle (`tooling/oracle`, TypeScript): rereads the WAV file of every case, recomputes its hash with Node's own SHA-256 and checks each channel against its mathematical definition (`conformance/expectations.json`); `engine-cli export` writes those files; both run in `cargo xtask check`.
- `@poyetik/config` (`tooling/config`): the strict TypeScript and ESLint configuration shared by every TypeScript package.
- `xtask`: Miri runs the tests of the engine FFI; the `bindings/wasm` package checks (build, types, lint, 100% coverage, mutation testing, end-to-end tests) are part of `cargo xtask check`.
- `xtask`: `cargo xtask check`, the single quality gate run by the pre-push hook and CI.
- Monorepo layout: `spec/`, `crates/`, `bindings/`, `apps/`, `conformance/`, with one Cargo workspace and one pnpm workspace at the root.
- Public roadmap (`apps/roadmap/`): kanban stored in `board.json`, driven by a CLI, rendered as a static page with collapsible steps.
- Project rules in `CLAUDE.md`.

### Changed

- **Breaking:** conformance expectations no longer declare a tolerance (`max_error`, refused if present). The oracle derives the largest allowed distance from the ideal sine, term by term (binary32 output rounding, binary64 phase accumulation over the case length, evaluation), and computes its reference with an exact phase reduction, so it stays exact however long the case (up to 2^29 frames). The bound went from a guessed 1e-6 to 2.98e-8 at 1 s; a phase computed in binary32 (1.7e-3) or a gain off by 5e-7 is rejected. The `osc~` tests use the same bound.
- **Breaking:** the patch of each conformance case is its own file, `conformance/patches/<name>.json`, and every runner (native CLI, Node tests, browser bench) passes its bytes to the engine unchanged. The runners used to rewrite the patch from a parsed copy, which could hide differences (a repeated parameter was silently merged; JavaScript writes `-0` as `0`). `engine-cli render` takes the case and the patch files. A case without its patch, or a patch without its case, is an error.
- The playground plays exactly the sine-440 patch of the conformance corpus (checked by a test), whose render has a reference hash.
- **Breaking:** `Patch` is a validated type, built only by `Patch::new`: node ids are unique, cables are never repeated, and a patch holds at most 65,536 nodes and 262,144 cables. The cables into an inlet are summed in a fixed order (source id, then outlet), whatever the order of the document. The patch reader also refuses documents over 16 MiB and a parameter named twice in one node.
- **Breaking:** `Plan::render` continues exactly where the previous call stopped (rendering `a` then `b` frames equals rendering `a + b`), and returns `CoreError::RenderTooLong` instead of failing when the samples cannot be held in memory.
- Execution order is computed with Kahn's algorithm, in O((N + E) log N) instead of O(N²·E): a patch of 16,000 boxes is ordered in milliseconds instead of seconds. The order is unchanged.
- Error messages name the faulty element once and say what is expected ("node 2 is declared twice", "invalid sample rate: 5 Hz (supported: 8000 to 384000 Hz)").
- The conformance CLI refuses a case whose patch has no audio output, instead of rendering nothing.
- The box laws (`engine-testkit`) run at every sample rate of a shared test matrix (8 kHz to 384 kHz, including 44.1 kHz), with special inputs (signed zeros, subnormals), a long run and parameters away from their default; a box that fails at any of them fails the laws. The matrix is also published as `conformance/test-matrix.json` for the TypeScript tests.

### Fixed

- `WasmEngine.render` no longer allocates: 500,000 renders cause no garbage collection (the previous version caused 100), checked by `test:allocation`, run without instrumentation.
- `WasmEngine` reads every export as unsigned (addresses from 2 GiB on came back negative), checks the number of arguments of each export, explains a module that cannot be instantiated, turns a trap into an `EngineTrap` and never calls a trapped module again, refuses a sample rate that is not a whole number of hertz (48000.5 was silently truncated) and outputs of different lengths (before rendering anything).
- The oracle computes ideal sines at the binary32 frequency the engine plays, as the patch format now states (parameters are binary32): an expectation at 440.1 Hz no longer compares with a frequency the patch cannot express.
- End-to-end tests run one browser at a time: in parallel, on an idle machine slowed down by the system, live captures and clicks timed out at random.
- `interleave` (engine-wasm hash helpers) copies samples bit for bit: a signaling NaN was turned quiet on the way to the hash.
- The determinism ban list covers every `f32`/`f64` method that is not correctly rounded (`atan2`, `exp2`, `hypot`, `cbrt`… were missing), and states the real reason for banning `mul_add`.
- `osc~` keeps its phase within one cycle at any frequency and sample rate: above the sample rate (possible from 8 kHz to 16 kHz), the phase grew without bound and the output drifted into noise within hours. Rendered samples are unchanged below the sample rate.
- Fresh clones install the conformance oracle's dependencies: `tooling/oracle` is now a pnpm workspace member (a green local check depended on leftovers).
- `cargo xtask check` rejects a coverage report that measured nothing (0 of 0 lines, functions or branches).
- The conformance export directory is emptied before every export, so no WAV file from an earlier run is judged.
- Mutation testing copies only versioned files (`--gitignore=true`).
