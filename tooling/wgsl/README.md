# tooling/wgsl

The WGSL proof suite: tools that hold WebGPU shaders to the bar the rest of the code meets
(validated and run on wgpu now, compared with a reference from roadmap #44 step 4; then mutation, coverage, lint and
formatting, each a card of its own: #45 step 1, #46 step 1, #47 step 1, #48 step 1). Each tool is a crate of its
own, built on naga (the WGSL compiler and validator of wgpu, used as is, never reimplemented),
depending on no other part of the repository, so that each can be published and used alone.

These crates form a Cargo workspace of their own (`Cargo.toml` here, with its `Cargo.lock`
and its `deny.toml`): their tree turns on features of crates the engine shares, which one
workspace would unify into the engine's builds, and holds licenses the root's policy does not
allow. Its package fields and lints are exactly the root's, its supply-chain policy derived
from the root's, and its naga the very version the GPU renderer's wgpu uses (each checked by
`cargo xtask check`).

| Crate | Contents |
|---|---|
| `wgsl-test` | Validates a WGSL source with naga (a thin layer: naga does the parsing and the validation, as its own `naga-cli` would; the layer pins wgpu's naga, sets the rules and capabilities, and shapes the report), every rule checked under naga's default capabilities (multisampled shading and cube array textures), the first problem reported on one line, made safe to show as every command-line tool of the project does, at naga's narrowest position; its command line, `wgsl-test validate <file>...` (`wgsl-test help`), does so for files, and `cargo xtask check` runs it on every tracked shader. Runs a compute kernel from a test (`Kernel`): the buffers it binds read from the shader, each buffer given checked against its binding before anything reaches the GPU, one dispatch, every buffer the kernel may write read back bit for bit, every failure a typed error (`RunError`), on one device shared by the tests of a program (`Gpu::shared`, the APIs of wgpu's `WGPU_BACKEND`), under WebGPU's default limits, which every browser grants, so that a kernel the tests prove runs on the web; on every adapter of the machine when a test asks (`Gpu::every`), headless, a CI runner's software GPU included. Compares what it wrote with a reference (`Outputs::check`, `compare`), bit for bit or within a tolerance carried with its derivation, and reports the first difference: its index, the adapter that computed it, both values and their bits, and the inputs the test describes there |

## Checks

`cargo xtask check` checks this workspace as it checks the root's: formatting, lints,
documentation, tests, 100% line and branch coverage of each crate by its own unit tests, no
surviving mutant, and the supply chain. To run its tests alone, from the root:

```sh
cargo test --manifest-path tooling/wgsl/Cargo.toml --workspace --locked -- --include-ignored
```
