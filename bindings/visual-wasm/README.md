# visual-wasm

The visual library compiled to WebAssembly (the `visual-wasm` crate, `src/`), and the TypeScript that drives it (`@poyetik/visual-wasm`). The module exports a minimal C interface with no imports (`src/ffi.rs`: its one audited `unsafe` file, run under Miri); `ts/visual.ts` checks a module once (its memory, every function and its number of arguments, the interface version), sets its square up when it is created, and drives it through typed methods, copying what it reads: a refused square throws the module's explanation naming the value refused, and any failed call into the module (a trap, out of memory among them, or a host stack overflow) kills the instance; a wrapper drives its instance alone. An instance's memory grows to hold the largest image set up, up to 512 MiB, and never shrinks: only a fresh instance gives it back.

| Path | What |
|---|---|
| `src/` | The crate: the safe view that renders and hashes the square, and the C interface |
| `ts/visual.ts` | `VisualModule`: instantiates and checks a module with its square, sets another one up, renders an image, hashes a sequence |
| `scripts/` | `pnpm build:wasm` and `pnpm test:reproducible`, wired to `@poyetik/wasm-build` with the visual module |
| `test/` | The interface against fake exports, and the conformance: every case of `conformance/visual/cases` rendered by the real module gives its reference hash, as the native renderer does |

```sh
pnpm check   # build the module, types, lint, unit and conformance tests at 100% coverage, reproducible build
```
