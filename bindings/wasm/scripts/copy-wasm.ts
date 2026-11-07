// Copies the engine module next to the compiled TypeScript: dist/engine.wasm.
import { copyFileSync } from "node:fs";

copyFileSync(
  new URL("../../../target/wasm32-unknown-unknown/release/engine_wasm.wasm", import.meta.url),
  new URL("../dist/engine.wasm", import.meta.url),
);
