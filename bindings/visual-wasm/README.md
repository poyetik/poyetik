# visual-wasm

The visual library compiled to WebAssembly (the `visual-wasm` crate, `src/`), and the TypeScript that drives it (`@poyetik/visual-wasm`). The module exports a minimal C interface with no imports (`src/ffi.rs`: its one audited `unsafe` file, run under Miri); `ts/visual.ts` checks a module once (its memory, every function and its number of arguments, the interface version), sets its square up when it is created, and drives it through typed methods, copying what it reads: a refused square throws the module's explanation naming the value refused, and any failed call into the module (a trap, out of memory among them, or a host stack overflow) kills the instance; a wrapper drives its instance alone. An instance's memory grows to hold the largest image set up, whose values take up to 512 MiB (the memory a little more), and never shrinks: only a fresh instance gives it back.

| Path | What |
|---|---|
| `src/` | The crate: the C interface (`ffi.rs`, its one audited `unsafe` file, run under Miri) and its test; the safe view it forwards to is the `visual-host` crate (`crates/visual-host`), tested natively |
| `ts/visual.ts` | The package's interface, `VisualModule`: creates its own instance from a compiled module (as a worker receives one) or from its bytes, so that no other code drives it, checks it with its square, sets another one up, renders an image, hashes a sequence |
| `ts/driver.ts` | The driver of one instance of any module exporting the visual interface, this package's or a product module's (`@poyetik/web-page`), checking the C interface at the boundary (the module's memory, functions, version and every answer); exported as `./driver` for those wrappers, which create the instance they drive |
| `scripts/` | `pnpm build:wasm` and `pnpm test:reproducible`, wired to `@poyetik/wasm-build` with the visual module |
| `benchmarks/` | The cost benchmarks of the module (`spec/benchmarks.md`, "Visual library"): render, and render and hash, per image size, counted by `@poyetik/wasm-build`'s instrumenter, and their law (an image costs its pixels) |
| `test/` | The interface against fake exports (contract checks, traps and failed calls); the conformance: every case of `conformance/visual/cases` rendered by the real module gives its reference hash, as the native renderer does, and every value of the square across the visual test matrix is its exact coverage; the largest image and its release; and the build commands run as processes (usage, failures, a closed stdout) |

```sh
pnpm check   # build the module, types, lint, unit and conformance tests at 100% coverage, reproducible build
```
