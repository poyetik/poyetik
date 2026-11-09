# engine-wasm

The engine in browsers: a WebAssembly module (Rust crate `engine-wasm`) and its TypeScript glue (package `@poyetik/engine-wasm`). The package exports the browser API (`./web`, `./hash`, `./worklet`, `./engine.wasm`); a Node entry point comes with the SDK (roadmap ticket #32), and the unit tests drive the module in Node through `ts/engine.ts` directly.

| Path | What |
|---|---|
| `src/` | The Rust crate: safe `Engine` and `Instance`, and `ffi`, the exported C interface (the only `unsafe` of the engine, checked with Miri) |
| `ts/engine.ts` | `WasmEngine`: one module instance, renders into buffers of any length, never ahead |
| `ts/worklet/processors.ts` | The `AudioWorklet` processors: the engine, and a tap that posts its input to the main thread |
| `ts/web.ts` | Main-thread API: registering the processors, loading a patch, playing it in engine nodes, tapping and capturing what a Web Audio node outputs, rendering offline |
| `ts/hash.ts` | The canonical render hash in TypeScript, independent of the Rust one |
| `test/` | Unit tests (Node, with the real module): every conformance case renders with its native reference hash |
| `e2e/` | End-to-end tests: a test bench page rendering conformance cases offline and live in Chromium, Firefox and Playwright's WebKit (desktop and iPhone sizes; not Safari) |

```sh
pnpm check   # build, types, lint, unit tests at 100% coverage, no allocation while rendering, reproducible build, end-to-end tests
```

The engine output is the patch's samples, unclipped: a level above 1.0 is kept, in the render hash and in an offline render, while a browser's audio output clips it. The final output stage belongs to the host, until the engine defines its own output policy (roadmap ticket #12).

The live end-to-end test runs a real `AudioContext`, muted: the engine output is captured before a zero gain.
The test bench (`e2e/site/`) shows its results as text, for Claude in Chrome too. The end-to-end tests start it with `pnpm bench` on a free port; to open it yourself, choose a port:

```sh
pnpm build && ENGINE_WASM_E2E_PORT=4392 pnpm bench
```

then open http://localhost:4392/e2e/site/index.html (the server answers only to `localhost` and `127.0.0.1` on that port).
