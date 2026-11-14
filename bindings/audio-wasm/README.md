# audio-wasm

The engine in browsers: a WebAssembly module (Rust crate `audio-wasm`) and its TypeScript glue (package `@poyetik/audio-wasm`). The package exports the browser API (`./web`, `./hash`, `./worklet`, `./engine.wasm`); a Node entry point comes with the SDK (#32 step 15), and the unit tests drive the module in Node through `ts/engine.ts` directly.

| Path | What |
|---|---|
| `src/` | The Rust crate: safe `Engine` and `Instance`, and `ffi`, the exported C interface (the only `unsafe` of this package, checked with Miri) |
| `ts/engine.ts` | `WasmEngine`: one module instance, renders into buffers of any length, never ahead, in one render format: binary64 into `Float64Array`s, binary64 given as binary32 or binary32 into `Float32Array`s |
| `ts/abi.ts` | The one reader of the module's exported interface (its memory, its version, each function with its arity), shared by `WasmEngine` and the benchmarks' counted instance |
| `ts/worklet/processors.ts` | The `AudioWorklet` processors: the engine, and a tap that posts its input to the main thread |
| `ts/web.ts` | Main-thread API: registering the processors, loading a patch in a precision (binary64 by default, binary32 as a setting; Web Audio is always given binary32), playing it in engine nodes, tapping and capturing what a Web Audio node outputs, rendering offline |
| `ts/hash.ts` | The canonical render hash (`render-hash/2`) in TypeScript, independent of the Rust one |
| `test/` | Unit tests (Node, with the real module): every conformance case renders with its native reference hash, in every render format |
| `benchmarks/` | The cost benchmarks (`spec/benchmarks.md`): an instrumenter counting the WebAssembly instructions the module executes and the words its bulk memory instructions move, the benchmarks of every box in both precisions, of the plan's sums, of whole patches played and of building plans, the module's size, and their comparison with the baseline |
| `e2e/` | End-to-end tests: a test bench page rendering conformance cases offline and live, in both precisions, in Chromium, Firefox and Playwright's WebKit (desktop and iPhone sizes; not Safari); the page's parameters are listed in `e2e/site/harness.ts` |

```sh
pnpm check   # build, types, lint, unit tests at 100% coverage, benchmarks, no allocation while rendering, reproducible build, end-to-end tests
```

The engine output is the patch's samples, unclipped: a level above 1.0 is kept, in the render hash and in an offline render, while a browser's audio output clips it. The final output stage belongs to the host, until the engine defines its own output policy (#12 step 9).

The live end-to-end test runs a real `AudioContext`, muted: the engine output is captured before a zero gain.
The test bench (`e2e/site/`) shows its results as text, for Claude in Chrome too. It is a test bench, not a product page: it has no accessibility audit. Its query parameters, buttons and outputs, and `window.timeRendering`, are listed at the top of `e2e/site/harness.ts`. The end-to-end tests start it with `pnpm bench` on a free port; to open it yourself, choose a port:

```sh
pnpm build && ENGINE_WASM_E2E_PORT=4392 pnpm bench
```

then open http://localhost:4392/e2e/site/index.html?case=sine-440&precision=binary32 (the server answers only to `localhost` and `127.0.0.1` on that port).
