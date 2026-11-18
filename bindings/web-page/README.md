# web-page

The product's module for the page of a browser (the `web-page` crate, `src/`), and the TypeScript that drives it (`@poyetik/web-page`). Each thread of the product loads one module holding what it runs (`spec/engine-design.md`, decision 33): the page's links the visual library and the coordinating layer, so that it never loads two modules side by side. The module exports the visual library's C interface (linked whole from `visual-wasm`) and the coordinating layer's own (`src/ffi.rs`, integers only: which image the page shows at a frame of the audio clock).

| Path | What |
|---|---|
| `src/` | The crate: `lib.rs` links the visual library's interface, `ffi.rs` exports the coordinating layer's (an audited `unsafe` file, run under Miri) |
| `ts/page.ts` | The package's interface, `PageModule`: creates its own instance from a compiled module or from its bytes, drives the visual interface as `@poyetik/visual-wasm` does, and answers which image the page shows at a frame (`imageAt`) |
| `ts/interface.ts` | Internal: the coordinating layer's interface, checked at the boundary (its version, its function, its 64-bit answer) |
| `scripts/` | `module.ts` names the module; `build-wasm.ts` and `reproducible.ts` build it into `dist/page.wasm` and prove the build reproducible (`@poyetik/wasm-build`) |
| `test/` | The real module, both interfaces, and the coordinating layer's checks on fake exports |

```sh
pnpm build             # build dist/page.wasm, then the package to JavaScript with its types (dist/*.js, *.d.ts), what its exports name
pnpm test              # tests, 100% coverage
pnpm test:reproducible # two builds, the same bytes
pnpm check             # all of the above, with typecheck and lint
```
