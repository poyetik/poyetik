# engine-wasm

The engine in browsers and in Node: a WebAssembly module (Rust crate `engine-wasm`) and its TypeScript glue (package `@poyetik/engine-wasm`).

| Path | What |
|---|---|
| `src/` | The Rust crate: safe `Engine` and `Instance`, and `ffi`, the exported C interface (the only `unsafe` of the engine, checked with Miri) |
| `ts/engine.ts` | `WasmEngine`: one module instance, renders into buffers of any length, never ahead |
| `ts/worklet/processors.ts` | The `AudioWorklet` processors: the engine, and a tap that posts its input to the main thread |
| `ts/web.ts` | Main-thread API: `loadPatch`, `createEngineNode`, `capture`, `renderOffline` |
| `ts/hash.ts` | The canonical render hash in TypeScript, independent of the Rust one |
| `test/` | Unit tests (Node, with the real module): sine-440 renders with the native reference hash |
| `e2e/` | End-to-end tests: a test bench page rendering conformance cases offline and live in Chromium, Firefox, WebKit and mobile WebKit |

```sh
pnpm check   # build, types, lint, unit tests at 100% coverage, mutation testing, end-to-end tests
```

The live end-to-end test runs a real `AudioContext`, muted: the engine output is captured before a zero gain.
The test bench (`e2e/site/`) shows its results as text, for Claude in Chrome too: `node e2e/serve.ts`, then open http://localhost:4392/e2e/site/index.html.
