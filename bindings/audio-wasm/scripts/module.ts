/** The engine's WebAssembly modules, as `@poyetik/wasm-build` builds them into `dist/`. */
import type { Modules } from "@poyetik/wasm-build/cli";
import type { WasmModule } from "@poyetik/wasm-build/wasm";
import { ENGINE_BLOCKS, type EngineBlock } from "../ts/blocks.ts";

/** The engine block of the module also copied to the package's default name, `engine.wasm`: `audio_core::BLOCK_SIZE`. */
const DEFAULT_ENGINE_BLOCK: EngineBlock = 64;

/**
 * The engine module of the engine block of `block` frames, built by the `audio-wasm` crate
 * for the size its build names (`POYETIK_ENGINE_BLOCK`; a module built for a size outside
 * `audio_core::ENGINE_BLOCKS` does not compile, and each module reports its own,
 * `engine_block_size`, checked by the unit tests) and copied to `dist/engine-<block>.wasm`,
 * the default size's to `dist/engine.wasm` too.
 */
function engineModule(block: EngineBlock): WasmModule {
  return {
    package: "audio-wasm",
    label: `engine module of ${String(block)}-frame engine blocks`,
    file: `engine-${String(block)}.wasm`,
    ...(block === DEFAULT_ENGINE_BLOCK ? { copies: ["engine.wasm"] } : {}),
    env: { POYETIK_ENGINE_BLOCK: String(block) },
  };
}

/** The smallest engine block, and the others. */
const [SMALLEST, ...LARGER] = ENGINE_BLOCKS;

/** The engine modules, one per engine block of `ENGINE_BLOCKS` (`ts/blocks.ts`), smallest first. */
export const ENGINE_MODULES: Modules = [engineModule(SMALLEST), ...LARGER.map(engineModule)];
