/** The engine's WebAssembly modules, as `@poyetik/wasm-build` builds them into `dist/`. */
import type { Modules } from "@poyetik/wasm-build/cli";
import type { WasmModule } from "@poyetik/wasm-build/wasm";

/**
 * The engine blocks a module is built for, in frames: `audio_core::ENGINE_BLOCKS`
 * (`spec/engine-design.md`, decision 2), one module each; a module built for another size
 * does not compile, and each module reports its own (`engine_block_size`, checked by the
 * end-to-end tests).
 */
const ENGINE_BLOCKS = [16, 32, 64, 128, 256, 512, 1024] as const;

/** The engine block of the module also copied to the package's default name, `engine.wasm`: `audio_core::BLOCK_SIZE`. */
const DEFAULT_ENGINE_BLOCK = 64;

/**
 * The engine module of the engine block of `block` frames, built by the `audio-wasm` crate
 * for the size its build names (`POYETIK_ENGINE_BLOCK`) and copied to
 * `dist/engine-<block>.wasm`, the default size's to `dist/engine.wasm` too.
 */
function engineModule(block: number): WasmModule {
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

/** The engine modules, one per engine block, smallest first. */
export const ENGINE_MODULES: Modules = [engineModule(SMALLEST), ...LARGER.map(engineModule)];
