# crates

The **engine core** and the project tooling, as Rust crates of one Cargo workspace.

| Crate | Contents |
|---|---|
| `audio-core` | Box contract, strong types, the math layer (the sample type, binary64 or binary32, and the math functions every box computes with), deterministic execution plan (`no_std`, no `unsafe`) |
| `audio-boxes` | The boxes (`osc~`, `dac~`), each written once for both precisions, and their registry (`no_std`, no `unsafe`, no math function but the math layer's) |
| `audio-battery` | The fingerprint battery (`spec/engine-design.md`, decision 29): the shared test matrix, the parameter sets and special inputs every box is tried with, and the fingerprint of a box type's outputs over them, which `audio-cli` records and the box laws share (no `unsafe`) |
| `audio-testkit` | Test bench, for tests only: the laws every registered box must respect, and the objective measurements of a signal (`spec/measurements.md`) (no `unsafe`) |
| `alloc-counter` | The global allocator counting heap operations (allocations and releases) in tests: the only `unsafe` of the test bench, audited and checked by Miri |
| `audio-fpenv` | Runs native engine calls in the default floating-point mode of the processor (no flush-to-zero, round to nearest) whatever mode the host set, restoring the host's mode after: native only (`x86_64`, aarch64), its one `unsafe` file reads and writes the control register with inline assembly, proven by its tests and audited (Miri cannot run assembly) |
| `audio-patch` | Reads the JSON patch format (`spec/patch-format.md`) (`no_std`, no `unsafe`) |
| `audio-hash` | Canonical SHA-256 hash of a render, `render-hash/2`: its render format and arithmetic mode, then its samples (`no_std`, no `unsafe`) |
| `audio-cli` | Offline renderer: renders conformance cases in every render format to render hashes and WAV files, and checks or rewrites the reference hashes (commands: `audio-cli help`) |
| `visual-core` | The visual library's core, standalone (it depends on no other part): images, exact time, the visual box contract, the square, and the exact CPU reference renderer (`no_std`, no `unsafe`) |
| `visual-hash` | Canonical SHA-256 hash of a sequence of images (`spec/image-hash.md`) (`no_std`, no `unsafe`) |
| `xtask` | Project automation: `cargo xtask check` and the git hooks' commands (commands: `cargo xtask help`) |

The engine compiled to WebAssembly, `audio-wasm`, is in `bindings/audio-wasm`.
