# visual-wasm

The visual library compiled to WebAssembly (the `visual-wasm` crate, `src/`), and the TypeScript that drives it (`@poyetik/visual-wasm`). The module exports a minimal C interface with no imports (`src/ffi.rs`: its one audited `unsafe` file, run under Miri); `ts/visual.ts` checks a module once (its memory, every function, the interface version) and drives it through typed methods, copying what it reads.

| Path | What |
|---|---|
| `src/` | The crate: the safe view that renders and hashes the square, and the C interface |
| `ts/visual.ts` | `VisualModule`: instantiates and checks a module, sets the square up, renders an image, hashes a sequence |
| `scripts/` | `pnpm build:wasm` and `pnpm test:reproducible`, wired to `@poyetik/wasm-build` with the visual module |
| `test/` | The interface against fake exports, and the conformance: every case of `conformance/visual/cases` rendered by the real module gives its reference hash, as the native renderer does |

```sh
pnpm check   # build the module, types, lint, unit and conformance tests at 100% coverage, reproducible build
```
