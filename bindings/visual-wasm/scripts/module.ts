/** The visual library's WebAssembly module, as `@poyetik/wasm-build` builds it into `dist/`. */
import type { WasmModule } from "@poyetik/wasm-build/wasm";

/** The visual module: built by the `visual-wasm` crate, copied to `dist/visual.wasm`. */
export const VISUAL_MODULE: WasmModule = { package: "visual-wasm", label: "visual module", file: "visual.wasm" };
