/** The engine's WebAssembly module, as `@poyetik/wasm-build` builds it into `dist/`. */
import type { WasmModule } from "@poyetik/wasm-build/wasm";

/** The engine module: built by the `audio-wasm` crate, copied to `dist/engine.wasm`. */
export const ENGINE_MODULE: WasmModule = { package: "audio-wasm", label: "engine module", file: "engine.wasm" };
