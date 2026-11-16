/** The page's WebAssembly module, as `@poyetik/wasm-build` builds it into `dist/`. */
import type { WasmModule } from "@poyetik/wasm-build/wasm";

/** The page module: built by the `web-page` crate, copied to `dist/page.wasm`. */
export const PAGE_MODULE: WasmModule = { package: "web-page", label: "page module", file: "page.wasm" };
