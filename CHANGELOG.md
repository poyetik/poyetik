# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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
