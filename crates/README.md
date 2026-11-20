# crates

The **two libraries** (the audio library and the visual library) and the project tooling, as Rust crates of one Cargo workspace, but the GPU renderer, `visual-gpu`, in a workspace of its own.

| Crate | Contents |
|---|---|
| `audio-core` | Box contract, strong types, the math layer (the sample type, binary64 or binary32, and the math functions every box computes with), deterministic execution plan and its execution modes, the output stage with its true-peak limiter, the settings a host states (`no_std`, no `unsafe`) |
| `audio-boxes` | The boxes (`osc~`, `dac~`), each written once for both precisions, and their registry (`no_std`, no `unsafe`, no math function but the math layer's) |
| `audio-battery` | The fingerprint battery (`spec/engine-design.md`, decision 29): the shared test matrix, the parameter sets and special inputs every box is tried with, and the fingerprint of a box type's outputs over them, which `audio-cli` records and the box laws share (no `unsafe`) |
| `audio-testkit` | Test bench, for tests only: the laws every registered box must respect, and the objective measurements of a signal (`spec/measurements.md`) (no `unsafe`) |
| `alloc-counter` | The global allocator counting heap operations (allocations and releases) in tests: the only `unsafe` of the test bench, audited and checked by Miri |
| `fpenv` | Runs native calls of either library in the default floating-point mode of the processor (no flush-to-zero, round to nearest) whatever mode the host set, restoring the host's mode after, and forces a host's mode for the tests of either library (`fpenv::host`): native only (`x86_64`, aarch64), its one `unsafe` file reads and writes the control register with inline assembly, proven by its tests and audited (Miri cannot run assembly) |
| `audio-patch` | Reads the JSON patch format (`spec/patch-format.md`) (`no_std`, no `unsafe`) |
| `audio-hash` | Canonical SHA-256 hash of a render, `render-hash/2`: its render format and arithmetic mode, then its samples (`no_std`, no `unsafe`) |
| `audio-host` | The engine as a host drives it: a patch loaded from its bytes, rendered block by block in its render format through the output stage, and the state a C interface forwards to; the WebAssembly module is a thin C interface over it (`no_std`, no `unsafe`) |
| `audio-cli` | Offline renderer: renders conformance cases and the live vector in every render format to render hashes, the cases to WAV files too, and checks or rewrites the reference hashes (commands: `audio-cli help`) |
| `visual-core` | The visual library's core, standalone (it depends on no other part): images, exact time, the visual box contract and the exact CPU reference renderer (`no_std`, no `unsafe`) |
| `visual-boxes` | The visual boxes (the square), each checked against the visual box laws (`no_std`, no `unsafe`) |
| `visual-hash` | Canonical SHA-256 hash of a sequence of images (`spec/image-hash.md`) (`no_std`, no `unsafe`) |
| `visual-host` | The square a host shows, set up from raw values and rendered or hashed without the heap: what the visual library's C interface forwards to, tested natively (`no_std`, no `unsafe`) |
| `visual-gpu` | The visual library on the GPU through wgpu (Metal, Vulkan, DirectX 12): the square rendered by its WGSL shader, bit for bit the CPU reference's image, and any image encoded for display by the display shader, byte for byte the CPU's encoding, each in bands when an image exceeds the GPU's buffers (no `unsafe`). A Cargo workspace of its own: wgpu's tree turns on features of crates the engine shares, which one workspace would unify into the engine's builds (its `Cargo.toml` says why; `cargo xtask check` checks it as the root's) |
| `visual-cli` | Visual offline renderer: renders the visual conformance cases to image hashes, and checks or rewrites their references and the display corpus's bytes (commands: `visual-cli help`) |
| `visual-testkit` | Visual test bench: the laws every visual box must respect, over the shared visual test matrix (no `unsafe`) |
| `coordinator` | The coordinating layer above the two libraries: what spans sound and image, which neither library may hold; first, the image a visual sequence shows at each frame of the audio clock, exact (`no_std`, no `unsafe`) |
| `xtask` | Project automation: `cargo xtask check` and the git hooks' commands (commands: `cargo xtask help`) |

The engine compiled to WebAssembly, `audio-wasm`, is in `bindings/audio-wasm`; the visual library compiled to WebAssembly, `visual-wasm`, in `bindings/visual-wasm`.
