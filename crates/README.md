# crates

The **engine core** and the project tooling, as Rust crates of one Cargo workspace.

| Crate | Contents |
|---|---|
| `engine-core` | Box contract, strong types, the math layer (the sample type, binary64 or binary32, and the math functions every box computes with), deterministic execution plan (`no_std`, no `unsafe`) |
| `engine-objects` | The boxes (`osc~`, `dac~`), each written once for both precisions, and their registry (`no_std`, no `unsafe`, no math function but the math layer's) |
| `engine-testkit` | Test bench: the laws every registered box must respect, and the objective measurements of a signal (`spec/measurements.md`) (no `unsafe`) |
| `engine-alloc-counter` | The global allocator counting heap operations (allocations and releases) in tests: the only `unsafe` of the test bench, audited and checked by Miri |
| `engine-fpenv` | Runs native engine calls in the default floating-point mode of the processor (no flush-to-zero, round to nearest) whatever mode the host set, restoring the host's mode after: native only (`x86_64`, aarch64), its one `unsafe` file reads and writes the control register with inline assembly, proven by its tests and audited (Miri cannot run assembly) |
| `engine-patch` | Reads the JSON patch format (`spec/patch-format.md`) (`no_std`, no `unsafe`) |
| `engine-hash` | Canonical SHA-256 hash of a render, `render-hash/2`: its render format and arithmetic mode, then its samples (`no_std`, no `unsafe`) |
| `engine-cli` | Offline renderer: renders conformance cases in every render format to render hashes and WAV files, and checks or rewrites the reference hashes (commands: `engine-cli help`) |
| `xtask` | Project automation: `cargo xtask check` and the git hooks' commands (commands: `cargo xtask help`) |

The engine compiled to WebAssembly, `engine-wasm`, is in `bindings/wasm`.
