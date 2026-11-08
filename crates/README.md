# crates

The **engine core** and the project tooling, as Rust crates of one Cargo workspace.

| Crate | Contents |
|---|---|
| `engine-core` | Box contract, strong types, deterministic execution plan (`no_std`, no `unsafe`) |
| `engine-objects` | The boxes (`osc~`, `dac~`) and their registry (`no_std`, no `unsafe`) |
| `engine-testkit` | Test bench: the laws every registered box must respect (no `unsafe`) |
| `engine-alloc-counter` | The global allocator counting allocations in tests: the only `unsafe` of the test bench, audited and checked by Miri |
| `engine-patch` | Reads the JSON patch format (`spec/patch-format.md`) (`no_std`, no `unsafe`) |
| `engine-hash` | Canonical SHA-256 hash of a render (`no_std`, no `unsafe`) |
| `engine-cli` | Offline renderer: WAV files, render hashes, conformance `check` and `bless` |
| `xtask` | Project automation: `cargo xtask check` |

The engine compiled to WebAssembly, `engine-wasm`, is in `bindings/wasm`.
